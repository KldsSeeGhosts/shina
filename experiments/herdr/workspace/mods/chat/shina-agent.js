import { panes, spaces, run } from "herdr";
import { modPanes, readDoc } from "./place.js";
import { hostTerminal } from "./connection.js";
import { openAgentPicker, startThread } from "./agents.js";
import { validAgent, draftKey, mayReplaceEmptyThread } from "./agent-policy.js";

const draftOf = view => ({ text: view.composer.value, chips: view.composer.chips.map(chip => ({ ...chip })) });
function replaceable(view, place) {
  const mine = panes().filter(p => p.tab === place?.tab);
  const host = hostTerminal(view.chat)?.address;
  return mayReplaceEmptyThread({
    empty: view.isEmpty(), busy: view.store.working() || Boolean(view.store.state.dialog) || Boolean(view.handoff),
    subagent: Boolean(view.sub), pending: view.sending,
    paneCount: mine.filter(p => p.mod).length, hostCount: mine.filter(p => p.terminal).length,
    foreignTerminal: mine.some(p => p.terminal && p.terminal !== host),
  });
}
export function showThreadAgent(view, cx) {
  const place = view.findPlace();
  if (!place) return view.flash("This thread is still opening. Try again in a moment.", cx);
  if (view.sub) return view.flash("Choose an agent in a top-level thread.", cx);
  openAgentPicker("thread", { space: place.space, chat: view.chat, provider: view.providerId(), empty: replaceable(view, place) });
}
export function selectThreadAgent(view, command, cx) {
  const [, chat, provider] = command.split(":");
  if (chat !== view.chat || !validAgent(provider) || view.changingAgent || provider === view.providerId()) return;
  const place = view.findPlace();
  if (!place || view.sub) return;
  const draft = draftOf(view), fingerprint = draftKey(draft);
  const replace = replaceable(view, place);
  const tabName = () => spaces().find(space => space.id === place.space)?.tabs.find(tab => tab.id === place.tab)?.name;
  const title = tabName();
  view.changingAgent = replace;
  let next;
  try { next = startThread(place.space, provider, { cwd: view.cwd, draft, ...(replace ? { name: title } : {}) }); }
  catch (e) { view.changingAgent = false; return view.flash(String(e), cx); }
  if (!replace) return;
  // Create and identify the new pane before closing the old empty tab. Never
  // close a split, a started conversation, or a draft edited during the handoff.
  cx.spawn(async (cx) => {
    try {
      let found = false;
      for (let i = 0; i < 40; i++) {
        const doc = await readDoc();
        const target = modPanes(doc, "chat").find(p => p.props?.chat === next);
        if (target && panes().some(p => p.pane === target.pane)) { found = true; break; }
        await cx.sleep(100);
      }
      if (found && replaceable(view, place) && draftKey(draftOf(view)) === fingerprint && tabName() === title) {
        view.composer.clear();
        view.saveDraft();
        run("tab.close", { id: place.tab, confirmed: true });
      } else {
        view.flash("The new thread is open. This thread and its draft were kept.", cx);
      }
    } catch {
      view.flash("This thread was kept. Check the new thread before continuing.", cx);
    } finally {
      view.changingAgent = false;
      cx.notify();
    }
  });
}
