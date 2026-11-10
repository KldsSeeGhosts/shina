import { spaces, panes, shared, setShared, run, openOverlay, notify } from "herdr";
import { randomBytes } from "crypto";
import { defaultForSpace, updateSpaceDefault, validAgent } from "./agent-policy.js";
export { agentLabel, agentShortLabel, AGENT_OPTIONS } from "./agent-policy.js";

export function targetSpace(id) {
  const all = spaces(), home = shared("t3-home.space");
  if (id) return all.find(space => space.id === id && space.id !== home) ?? null;
  return all.find(space => space.active && space.id !== home)
    ?? all.find(space => space.id === shared("shina.space") && space.id !== home)
    ?? all.find(space => space.id !== home && !space.machine) ?? null;
}
export const spaceDefaultAgent = id => defaultForSpace(id, shared("shina.spaceAgents"));
export function saveSpaceDefault(id, agent) {
  const space = targetSpace(id);
  if (!space || space.machine) throw new Error("Choose a local space. Native remote chat is not connected yet.");
  setShared("shina.spaceAgents", updateSpaceDefault(shared("shina.spaceAgents"), id, agent));
}
export function openAgentPicker(scope = "new", options = {}) {
  const space = targetSpace(options.space);
  if (!space) return run("mod.open", { name: "t3-new-menu" });
  if (space.machine) return notify({ title: "Remote coding agents", body: "This space supports terminal agents. Native remote chat is not connected yet.", level: "warning" });
  return openOverlay({ mod: "shina-agent", kind: "popup", frame: "none", at: "top", offset: 96, width: 360, height: 330,
    props: { ...options, scope, space: space.id } });
}
export function startThread(spaceId, provider, options = {}) {
  const space = targetSpace(spaceId);
  if (!space || space.machine || !validAgent(provider)) throw new Error("Choose a local space and an installed coding agent.");
  const chat = `c-${randomBytes(8).toString("hex")}`;
  if (options.draft?.text || options.draft?.chips?.length) {
    setShared("shina.drafts", { ...(shared("shina.drafts") ?? {}), [chat]: options.draft });
  }
  setShared("shina.space", space.id);
  const cwd = options.cwd || panes().find(p => space.tabs.some(tab => tab.id === p.tab) && p.cwd)?.cwd || space.folder;
  run("tab.new", { space: space.id, name: options.name || "New thread", mod: "chat", props: { chat, provider, ...(cwd ? { cwd } : {}) } });
  return chat;
}
