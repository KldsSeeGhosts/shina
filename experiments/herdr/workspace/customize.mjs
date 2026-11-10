// Adapt pinned upstream files. Refuse drift instead of silently skipping a patch.
export function replaceOnce(source, before, after, label) {
  if (!source.includes(before) || source.indexOf(before) !== source.lastIndexOf(before)) {
    throw new Error(`Upstream changed at ${label}; review the integration before deploying.`);
  }
  return source.replace(before, after);
}

export function workspaceLayout(upstream, fonts = { sans: "Anthropic Sans Variable Text" }, localMachineName = "This computer") {
  const layout = structuredClone(upstream);
  layout.name = "Shina";
  layout.theme = { light: "shina-light", dark: "shina-dark" };
  layout.ui_font = fonts.sans;
  layout.font = "Menlo";
  layout.font_size = 13;
  layout.preview = "layouts/shina-preview.svg";
  layout.floating = [
    { id: "new-chat-commands", edge: "bottom", align: "start", content: { mod: "new-chat", props: { role: "commands" } } },
  ];
  const walk = (node) => {
    if (node.id === "sidebar") {
      node.size = { px: 280 };
      node.resizable = true;
    }
    if (node.id === "spaces") node.content.props.localMachineName = localMachineName;
    if (node.id === "main") {
      node.content.pane.background = "canvas";
      node.content.pane.border.color = "canvas";
      node.content.pane.focus.color = "canvas";
      node.content.pane.radius = 12;
    }
    if (node.id === "tabs-row") {
      node.children.splice(2, 0, {
        id: "shina-tools", type: "block", size: { px: 224 }, content: { mod: "shina-tools" },
      });
    }
    if (node.id === "work-body") {
      node.children.push(
        { id: "shina-files", type: "block", hidden: true, size: { px: 300 }, min: 220, max: 600, background: "canvas", radius: 12, content: { mod: "files" } },
        { id: "shina-changes", type: "block", hidden: true, size: { px: 380 }, min: 260, max: 800, background: "canvas", radius: 12, content: { mod: "changes" } },
      );
    }
    for (const child of node.children ?? []) walk(child);
  };
  walk(layout);
  return layout;
}

export function workspaceKeys(upstream, existing = {}) {
  const keys = structuredClone(existing);
  for (const [context, bindings] of Object.entries(upstream)) {
    const name = context === "layout:herdr-t3" ? "layout:shina-workspace" : context;
    keys[name] = { ...keys[name], ...bindings };
  }
  Object.assign(keys["layout:shina-workspace"], {
    "cmd-i": "new-chat.open",
    "cmd-alt-t": "new-chat.terminal",
    "cmd-shift-n": "new-chat.open",
    "cmd-shift-e": "shina-tools.files",
    "cmd-shift-d": "shina-tools.changes",
    "cmd-alt-a": "shina-tools.agent",
  });
  keys["mod:new-chat"] = { ...keys["mod:new-chat"], "cmd-w": "overlay.close", "cmd-shift-w": "overlay.close" };
  keys["mod:shina-agent"] = { "cmd-w": "overlay.close", "cmd-shift-w": "overlay.close" };
  return keys;
}

export function customize(relative, input, fonts = { sans: "Anthropic Sans Variable Text", serif: "Anthropic Serif Variable Text" }) {
  let text = input;
  const swap = (before, after) => { text = replaceOnce(text, before, after, relative); };
  if (relative === "t3-chrome/main.js") {
    swap('shared("layout") !== "herdr-t3"', 'shared("layout") !== "shina-workspace"');
    swap('/layouts/herdr-t3.json', '/layouts/shina-workspace.json');
    swap('.child("Herdr")', '.child("Shina")');
    swap('image("frame/welcome/herdr.png")', 'image("mods/shina-home/assets/ember.png")');
  }
  if (relative === "t3-tabs/navigation.js") {
    if (text.split('"t3-home"').length !== 4) throw new Error("Upstream Home navigation changed.");
    text = text.replaceAll('"t3-home"', '"shina-home"');
  }
  if (relative === "t3-pane/main.js") {
    swap('const named = p.mod === "t3-home" ? "" : namer()(p);',
      'const named = p.mod === "shina-home" ? "" : p.mod === "chat" ? namer()(agents().find(a => a.tab === p.tab) ?? p) : namer()(p);');
  }
  if (relative === "t3-spaces/main.js") {
    swap('import { View, div, svg } from "gpui-kit";',
      'import { View, div, svg, image } from "gpui-kit";\nimport { captainForMachine } from "./shina-crew.js";');
    swap('    if (!remote.length) return loading(null) && !rows.length ? [loadingRow(null)] : rows.map(entry);\n', "");
    swap('.child(svg(`mods/t3-spaces/icons/${m ? "server" : "monitor"}.svg`).size(14).ml(-1).flex_none())',
      '.child(image(`mods/t3-spaces/shina-assets/${captainForMachine(m, props()?.machineCaptains)}.png`).size(20).ml(-1).flex_none())');
    text = text.replaceAll('m ?? "This computer"', 'm ?? props()?.localMachineName ?? "This computer"');
  }
  if (relative.endsWith("/zoom.js")) {
    swap('import { panes, run } from "herdr";', 'import { panes, terminals, run, openOverlay } from "herdr";');
    swap('export function focusAgent(terminal) {\n', `export function focusAgent(terminal) {
  const host = terminals().find(t => t.address === terminal);
  if (host?.parent && host.labels?.["herdr-chat.chat"]) {
    openOverlay({ mod: "chat", kind: "dialog", width: 0.8, height: 0.8,
      props: { chat: host.labels["herdr-chat.chat"], peek: terminal, provider: host.labels["herdr-chat.provider"] ?? "pi" } });
    return;
  }
`);
  }
  if (relative.endsWith("/project.js")) {
    swap('import { image, svg } from "gpui-kit";', 'import { image, svg } from "gpui-kit";\nimport { spaceDefaultAgent, startThread } from "./agents.js";');
    swap(
      'export const newTab = (s) => run("tab.new", { space: s.id, folder: placeOf(s, panes()) });',
      `// Shina: local work opens as structured chat. Remote spaces retain working terminals.
export const newTab = (s) => {
  if (!s) return run("mod.open", { name: "t3-new-menu" });
  if (s.machine) return run("tab.new", { space: s.id, folder: placeOf(s, panes()) });
  return startThread(s.id, spaceDefaultAgent(s.id), { cwd: placeOf(s, panes()) });
};`,
    );
  }
  if (relative === "chat/main.js") {
    swap("const COL_MAX = 800;", "const COL_MAX = 820;");
    swap('import { View, div } from "gpui-kit";', 'import { View, div } from "gpui-kit";\nimport { submitDraft } from "./shina.js";\nimport { showThreadAgent, selectThreadAgent } from "./shina-agent.js";');
    swap('const t = theme();\n    const s = this.store.state;\n    const rows =', 'const t = { ...theme(), background: theme().canvas };\n    const s = this.store.state;\n    const rows =');
    swap('min_h(0).bg(t.background).text_color(t.foreground).text_size(13)', 'min_h(0).bg(t.background).font_family("Anthropic Sans Variable").font_normal().text_color(t.foreground).text_size(13)');
    swap('placeholder: `Message ${PROVIDER_NAMES[this.provider ?? DEFAULT_PROVIDER] ?? "the agent"}…`', 'placeholder: "What would you like to work on?"');
    swap('const ours = PROVIDERS.includes(tab.name) || tab.name === this.tabName || !tab.named;',
      'const ours = tab.name === "New thread" || PROVIDERS.includes(tab.name) || tab.name === this.tabName || !tab.named;');
    swap('onChange: (cx) => { this.updateMenu(cx); this.debug.mark(); cx.notify(); },',
      'onChange: (cx) => { this.saveDraft(); this.updateMenu(cx); this.debug.mark(); cx.notify(); },');
    swap('    this.conn = new Connection({', `    const draft = shared("shina.drafts")?.[this.chat];
    if (draft && !this.peekMode) {
      this.composer.set(draft.text ?? "");
      this.composer.chips = Array.isArray(draft.chips) ? draft.chips : [];
    }
    this.conn = new Connection({`);
    swap('    this.composer.focus();\n    cx.timer.after(400, () => this.composer.focus());', `    if (this.peekMode) {
      cx.bind_keys(["cmd-w", "cmd-shift-w", "ctrl-alt-x", "ctrl-alt-shift-x"].flatMap(keystroke =>
        ["ShinaChatPeek", "ShinaChatPeek > Input"].map(context => ({ keystroke, action: "shina_chat_close_peek", context }))));
    }
    this.composer.focus();
    cx.timer.after(400, () => this.composer.focus());`);
    swap('.key_context("HerdrChat").size_full()', '.key_context(this.peekMode ? "HerdrChat ShinaChatPeek" : "HerdrChat").on_action("shina_chat_close_peek", () => { if (this.peekMode) closeOverlay(props().overlay); }).size_full()');
    swap('  tick(cx) {\n', '  tick(cx) {\n    this.saveDraft();\n');
    swap('  close() {\n    if (this.viewer || this.termbar) return;', '  close() {\n    if (this.viewer || this.termbar) return;\n    this.saveDraft();');
    swap('  send(cx, mode = "auto") {\n', `  saveDraft() {
    if (!this.composer || this.peekMode) return;
    const value = { text: this.composer.value, chips: this.composer.chips };
    const fingerprint = JSON.stringify(value);
    if (fingerprint === this.draftFingerprint) return;
    this.draftFingerprint = fingerprint;
    const all = { ...(shared("shina.drafts") ?? {}) };
    if (value.text || value.chips.length) all[this.chat] = value;
    else delete all[this.chat];
    setShared("shina.drafts", all);
  }

  send(cx, mode = "auto") {
    if (this.changingAgent) return this.flash("Changing the coding agent. Your draft is kept.", cx);
    if (this.sending) return;
`);
    swap('      case "chip:model": return this.openModelPicker(cx);', '      case "chip:agent": return showThreadAgent(this, cx);\n      case "chip:model": return this.openModelPicker(cx);');
    swap('    if (c === "focus") this.focusInput(); else if', '    if (c.startsWith("agent-select:")) return selectThreadAgent(this, c, cx);\n    if (c === "focus") this.focusInput(); else if');
    swap(`    this.composer.clear();
    this.timeline.toBottom();
    this.fire("send", { text, ...(images.length ? { images } : {}), mode }, cx, (d) => {
      this.lastSend = { text, disposition: d?.disposition ?? null, at: Date.now() };
    });`,
    `    this.sending = true;
    this.saveDraft();
    this.timeline.toBottom();
    cx.spawn(async (cx) => {
      try {
        const { result: d, cleared } = await submitDraft({ text, images }, {
          send: (value) => this.req("send", { ...value, mode }, cx),
          read: () => ({ text: this.composer.value.replace(/\\s+$/, ""), images: this.composer.chips.map(c => c.path) }),
          clear: () => this.composer.clear(),
        });
        this.lastSend = { text, disposition: d?.disposition ?? null, at: Date.now() };
        if (!cleared) this.flash("Sent. Your edited draft is still here.", cx);
      } catch (e) {
        this.flash("Not sent. Your draft is still here. " + message(e), cx);
      } finally {
        this.sending = false;
        this.saveDraft();
        this.debug.mark();
        cx.notify();
      }
    });`);
  }
  if (relative === "chat/rich.js") {
    swap('import { SPACE } from "./space.js";', 'import { SPACE } from "./space.js";\nimport { displayText } from "./shina.js";');
    swap('export const prose = (id, text) => TextView.markdown(id, text).selectable(true);',
      `export const prose = (id, text, reading = false) => {
  const view = TextView.markdown(id, displayText(text)).selectable(true);
  return reading ? view.font_family("Anthropic Serif Variable").font_weight(400).text_size(16).line_height("25px") : view;
};`);
    swap('if (parts.length === 1) return prose(id, parts[0].text);', 'if (parts.length === 1) return prose(id, parts[0].text, true);');
    swap('child(prose(`${id}-${j}`, p.text))', 'child(prose(`${id}-${j}`, p.text, true))');
  }
  if (relative === "chat/composer.js") {
    swap('this.ta.set_auto_grow(1, 10);', 'this.ta.set_auto_grow(2, 10);');
    swap('.w_full().px(14).pt(12).pb(4).text_size(14)', '.w_full().px(18).pt(16).pb(8).text_size(15)');
    swap('.w(30).h(30).rounded(10)', '.w(32).h(32).rounded_full()');
    swap('.w_full().rounded(14).border_1().border_color(this.focused ? alpha(t.accent, 0.55) : t.input).bg(t.surface)',
      '.w_full().rounded(20).border_1().border_color(this.focused ? alpha(t.accent, 0.65) : t.input).bg(t.surface)');
    swap('.h(42).pl(8).pr(6).gap(4)', '.h(46).pl(12).pr(10).gap(4)');
  }
  if (relative === "chat/chrome.js") {
    swap('  const modelTip = withKey(', `  left.push(chipEl("chip-agent", t, { tip: "Coding agent for this thread", onClick: (_e, cx) => v.control("chip:agent", cx) })
    .when(tiny, e => e.px(6)).child(providerIcon(provider, 13))
    .when(!tiny, e => e.child(div().max_w(86).truncate().child(small ? provider === "claude" ? "Claude" : v.providerName() : v.providerName())))
    .child(chevron()));
  drawn.push("agent");
  const modelTip = withKey(`);
    swap('    .child(providerIcon(provider, 13))\n    .when(!tiny', '    .when(tiny, e => e.child(icon("cpu", 13)))\n    .when(!tiny');
    swap('const leftW = 20 +', 'const leftW = (tiny ? 36 : small ? 58 : 120) + 20 +');
    swap('tier === "md" ? 120 : 180).truncate()', 'tier === "md" ? 90 : 150).truncate()');
    swap('.text_size(28).font_semibold().child(v.providerName())',
      '.font_family("Anthropic Serif Variable").font_normal().text_size(32).child("What shall we work on?")');
    swap('.child(providerIcon(v.providerId(), 26))', '.child(providerIcon(v.providerId(), 22))');
    swap('.child(cwd));', '.child(`${v.providerName()} · ${cwd}`));');
  }
  if (relative === "new-chat/main.js") {
    swap('import { randomBytes } from "crypto";', 'import { randomBytes } from "crypto";\nimport { openAgentPicker, targetSpace, startThread } from "./agents.js";');
    swap('run("tab.new", { name: provider, mod: "chat", props: { chat, provider, ...(cwd ? { cwd } : {}) } });',
      `if (!space || space.id === shared("t3-home.space")) {
    const target = targetSpace();
    if (target) startThread(target.id, provider);
    else run("mod.open", { name: "t3-new-menu" });
  } else if (space.machine) {
    notify({ title: "Remote chat", body: "Native remote chat is not supported yet. Open a terminal in this machine's space.", level: "warning" });
  } else {
    startThread(space.id, provider, { cwd });
  }`);
    swap('readTextFile, run, spaces, terminals, theme }', 'readTextFile, run, shared, setShared, spaces, terminals, theme }');
    swap('if (c === "open") run("mod.open", { name: "new-chat" });',
      `if (c === "terminal") {
            const s = spaces().find(s => s.active && s.id !== shared("t3-home.space"))
              ?? spaces().find(s => s.id === shared("shina.space"));
            if (s) run("tab.new", { space: s.id, folder: s.folder });
            else run("shina-home.terminal");
          } else if (c === "open") openAgentPicker("new");`);
  }
  return text.replaceAll('"Anthropic Sans Variable"', JSON.stringify(fonts.sans))
    .replaceAll('"Anthropic Serif Variable"', JSON.stringify(fonts.serif));
}
