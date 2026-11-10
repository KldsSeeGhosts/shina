// Native agent-choice QA. Creates only its own spaces, sends no model prompts.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { readFile, writeFile, mkdir, mkdtemp, unlink } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { randomUUID } from "node:crypto";

const runtime = process.env.SHINA_HERDR_HOME || join(homedir(), "Library/Application Support/ShinaHerdr");
const profile = join(runtime, "profile");
const runner = fileURLToPath(new URL("run.mjs", import.meta.url));
const repo = fileURLToPath(new URL("../..", import.meta.url)).replace(/\/$/, "");
const json = async file => JSON.parse(await readFile(file, "utf8"));
function execute(args, input) {
  const result = spawnSync(process.execPath, [runner, ...args], { encoding: "utf8", input, timeout: 15_000 });
  assert.equal(result.status, 0, result.stderr || result.stdout || String(result.error));
  return result.stdout;
}
const call = (...args) => JSON.parse(execute(args));
function document() {
  const lines = [
    { jsonrpc: "2.0", id: 1, method: "hello", params: { generation: 1 } },
    { jsonrpc: "2.0", id: 2, method: "doc.get", params: { name: "herdr-app" } },
  ];
  const records = execute(["core", "connect"], lines.map(JSON.stringify).join("\n") + "\n").trim().split("\n").map(JSON.parse);
  return records.find(row => row.id === 2).result.value;
}
function chats(space) {
  const result = [];
  const walk = (tab, node) => {
    if (node?.content?.mod === "chat") result.push({ tab, pane: node.id, ...node.content.props });
    for (const child of node?.children ?? []) walk(tab, child);
  };
  for (const tab of document().spaces.find(s => s.id === space)?.tabs ?? []) walk(tab.id, tab.root);
  return result;
}
async function wait(read, match, description) {
  const end = Date.now() + 25_000;
  while (Date.now() < end) {
    try { const value = await read(); if (match(value)) return value; }
    catch (e) { if (e.code !== "ENOENT") throw e; }
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  throw new Error(`Timed out waiting for ${description}.`);
}
const state = () => json(join(profile, "shared.json"));
const debug = chat => json(join(profile, "data/herdr-chat/debug", `${chat}.json`));
const created = [];
const original = call("status");
const flag = join(profile, "data/herdr-chat/debug/enabled");
let debugWasOn = true;
try { await readFile(flag); } catch { debugWasOn = false; }
await mkdir(join(profile, "data/herdr-chat/debug"), { recursive: true });
await writeFile(flag, "Agent picker QA.\n");
async function picker(scope, space, current) {
  await wait(state, s => s["shina.agentPicker"]?.scope === scope && s["shina.agentPicker"]?.space === space
    && s["shina.agentPicker"]?.current === current && s["shina.agentPicker"]?.available?.codex, `${scope} agent picker`);
}
async function setDefault(space, id, before = "pi") {
  call("do", "space.select", JSON.stringify({ id: space }));
  call("do", "shina-tools.agent");
  await picker("space", space, before);
  call("do", `shina-agent.pick:${id}`);
  await wait(state, s => s["shina.spaceAgents"]?.[space] === id, `the ${id} space default`);
}
try {
  if (original.overlays.length) call("do", "overlay.close");
  const a = call("do", "space.new", JSON.stringify({ name: "Agent chooser QA A", folder: repo, argv: ["/bin/zsh", "-f"] })).space;
  created.push(a);
  const b = call("do", "space.new", JSON.stringify({ name: "Agent chooser QA B", folder: repo, argv: ["/bin/zsh", "-f"] })).space;
  created.push(b);
  await setDefault(a, "claude");
  await setDefault(b, "codex");
  assert.equal((await state())["shina.spaceAgents"][a], "claude", "space B must not change space A");
  assert.equal(chats(a).length + chats(b).length, 0, "choosing defaults does not create a thread");
  call("do", "space.select", JSON.stringify({ id: a }));
  call("do", "t3-tabs.new");
  const first = await wait(() => chats(a), rows => rows.length === 1, "a thread created from the space default");
  const old = first[0];
  assert.equal(old.provider, "claude");
  await wait(() => debug(old.chat), d => d.connection?.status === "connected" && d.empty && d.phase === "idle", "the pristine Claude thread");
  call("do", "tab.rename", JSON.stringify({ id: old.tab, name: "QA draft title" }));
  call("do", "chat.type:A draft that must survive changing the coding agent.");
  call("do", "chat.drop:" + join(repo, "assets/crew/ember/Normal.small.png"));
  const withImage = await wait(() => debug(old.chat), d => d.composer?.chips?.length === 1, "an attached image");
  call("do", "chat.click:chip:agent");
  await picker("thread", a, "claude");
  call("do", "shina-agent.pick:codex");
  const changed = await wait(() => chats(a), rows => rows.length === 1 && rows[0].provider === "codex" && rows[0].chat !== old.chat, "replacement of the empty thread");
  const next = changed[0];
  assert.equal(call("ls").spaces.find(space => space.id === a).tabs.find(tab => tab.id === next.tab).name, "QA draft title");
  const restored = await wait(() => debug(next.chat), d => d.composer?.text === withImage.composer.text && d.composer.chips.length === 1, "the transferred draft and image");
  assert.deepEqual(restored.composer.chips, withImage.composer.chips);
  assert.equal((await state())["shina.spaceAgents"][a], "claude", "a thread override must not change its space default");
  assert.equal((await state())["shina.spaceAgents"][b], "codex");

  // Pi's installed SessionManager uses format version 3. This fixture has
  // labeled example history, never a real prompt or a user's transcript.
  const folder = await mkdtemp(join(runtime, "agent-qa-"));
  const fixture = join(folder, "example-history.jsonl");
  const now = Date.now(), stamp = new Date(now).toISOString();
  const header = { type: "session", version: 3, id: randomUUID(), timestamp: stamp, cwd: repo };
  await writeFile(fixture, [
    header,
    { type: "message", id: "qa-user", parentId: null, timestamp: stamp, message: { role: "user", content: [{ type: "text", text: "Example history for agent-choice QA. No model prompt was submitted." }], timestamp: now } },
  ].map(JSON.stringify).join("\n") + "\n");
  const fixtureId = `c-qa-${randomUUID()}`;
  const history = call("do", "tab.new", JSON.stringify({ space: b, name: "Example history", mod: "chat", props: { chat: fixtureId, provider: "pi", cwd: repo, session: fixture } }));
  await wait(() => debug(fixtureId), d => d.connection?.status === "connected" && d.items?.some(item => item.kind === "user"), "the example history");
  call("do", "chat.click:chip:agent");
  await picker("thread", b, "pi");
  call("do", "shina-agent.pick:claude");
  await wait(() => chats(b), rows => rows.some(row => row.provider === "claude"), "a new agent thread beside existing history");
  assert.ok(chats(b).some(row => row.tab === history.tab && row.chat === fixtureId), "existing history must remain open and unchanged");
  assert.deepEqual(call("status").errors, []);
  console.log("Agent smoke passed: independent space defaults, default-based creation, per-thread override, draft/image transfer, and preservation of existing history. No model prompts were sent.");
} finally {
  call("do", "overlay.close");
  for (const id of created.reverse()) call("do", "space.close", JSON.stringify({ id, confirmed: true }));
  if (original.tab) call("do", "tab.select", JSON.stringify({ id: original.tab }));
  if (!debugWasOn) await unlink(flag);
}
