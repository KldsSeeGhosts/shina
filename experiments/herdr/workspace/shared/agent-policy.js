export const AGENT_OPTIONS = [
  { id: "pi", name: "Pi", short: "Pi" },
  { id: "claude", name: "Claude Code", short: "Claude" },
  { id: "codex", name: "Codex", short: "Codex" },
];
export const validAgent = id => AGENT_OPTIONS.some(agent => agent.id === id);
export const agentLabel = id => AGENT_OPTIONS.find(agent => agent.id === id)?.name ?? "Pi";
export const agentShortLabel = id => AGENT_OPTIONS.find(agent => agent.id === id)?.short ?? "Pi";
export const defaultForSpace = (space, preferences = {}) => validAgent(preferences?.[space]) ? preferences[space] : "pi";
export function updateSpaceDefault(preferences, space, agent) {
  if (!space || !validAgent(agent)) throw new Error("Choose a valid coding agent and space.");
  return { ...preferences, [space]: agent };
}
export const draftKey = draft => JSON.stringify({ text: draft?.text ?? "", chips: draft?.chips ?? [] });
export function mayReplaceEmptyThread({ empty, busy, subagent, paneCount, hostCount, pending, foreignTerminal }) {
  return empty === true && !busy && !subagent && !pending && !foreignTerminal && paneCount === 1 && hostCount <= 1;
}
