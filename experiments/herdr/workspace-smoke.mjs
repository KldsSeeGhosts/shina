// Native interaction checks in the private workspace. Sends no model prompts.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { readFile, writeFile, mkdir, utimes, unlink } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { randomUUID } from "node:crypto";

const runtime = process.env.SHINA_HERDR_HOME || join(homedir(), "Library/Application Support/ShinaHerdr");
const profile = join(runtime, "profile");
const runner = fileURLToPath(new URL("run.mjs", import.meta.url));
const readJson = async file => JSON.parse(await readFile(file, "utf8"));
function call(...args) {
  const result = spawnSync(process.execPath, [runner, ...args], { encoding: "utf8", timeout: 20_000 });
  assert.equal(result.status, 0, result.stderr || result.stdout || String(result.error));
  return args[0] === "open" ? null : JSON.parse(result.stdout);
}
async function wait(read, matches, label, timeout = 20_000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    try {
      const value = await read();
      if (matches(value)) return value;
    } catch (e) {
      if (e?.code !== "ENOENT") throw e;
    }
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  throw new Error(`Timed out waiting for ${label}.`);
}
const shared = () => readJson(join(profile, "shared.json"));
const settings = () => readJson(join(profile, "settings.json"));
const debug = chat => readJson(join(profile, "data/herdr-chat/debug", `${chat}.json`));
const original = call("ls");
const space = original.spaces.find(s => s.active && s.name !== "Home\u200b") ?? original.spaces.find(s => s.name !== "Home\u200b" && !s.machine);
assert.ok(space, "Open a local project first.");
assert.equal(call("status").layout, "shina-workspace");
const originalTab = original.spaces.find(s => s.active)?.tabs.find(t => t.active)?.id;
const originalAppearance = (await settings()).appearance;
const debugFlag = join(profile, "data/herdr-chat/debug/enabled");
let debugWasOn = true;
try { await readFile(debugFlag); } catch { debugWasOn = false; }
await mkdir(join(profile, "data/herdr-chat/debug"), { recursive: true });
await writeFile(debugFlag, "Shina native verification.\n");
const created = [];
const makeChat = async name => {
  const chat = `c-smoke-${randomUUID()}`;
  const made = call("do", "tab.new", JSON.stringify({ space: space.id, name, mod: "chat", props: { chat, provider: "pi", cwd: space.folder } }));
  created.push(made.tab);
  const state = await wait(() => debug(chat), d => d.connection?.status === "connected" && d.hostHidden, "a connected hidden host");
  assert.equal(state.renderError, null);
  return { chat, tab: made.tab, terminal: state.host.address };
};

try {
  const a = await makeChat("Shina QA draft A");
  call("do", "chat.type:A draft that belongs only to thread A.");
  await wait(shared, s => s["shina.drafts"]?.[a.chat]?.text === "A draft that belongs only to thread A.", "A's persisted draft");
  const b = await makeChat("Shina QA draft B");
  call("do", "chat.type:A separate draft in thread B.");
  await wait(shared, s => s["shina.drafts"]?.[b.chat]?.text === "A separate draft in thread B.", "B's persisted draft");
  call("do", "tab.select", JSON.stringify({ id: a.tab }));
  await wait(() => debug(a.chat), d => d.composer?.text === "A draft that belongs only to thread A.", "A's isolated composer");

  const timestamp = Date.now();
  await utimes(join(profile, "mods/chat/main.js"), new Date(), new Date());
  await wait(() => debug(a.chat), d => d.at > timestamp && d.composer?.text === "A draft that belongs only to thread A." && d.connection.status === "connected", "draft recovery after mod reload");

  call("do", "chat.click:chip:model");
  await wait(() => debug(a.chat), d => Boolean(d.picker), "the native model picker");
  call("do", "chat.outside");
  call("do", "t3-inbox.custom-snooze:" + a.terminal + ":" + (Date.now() + 3600_000));
  await wait(() => call("core", "get", a.terminal, "--json"), t => Number(t.labels?.snoozed) > Date.now(), "the inbox snooze label");
  call("core", "label", a.terminal, "snoozed=");
  call("do", "tab.select", JSON.stringify({ id: a.tab }));
  await wait(() => call("status"), s => s.tab === a.tab, "returning to the snoozed thread");
  call("do", "t3-tabs.settle");
  await wait(() => call("core", "get", a.terminal, "--json"), t => Boolean(t.labels?.settled), "the inbox settled label");
  await wait(() => call("status"), s => s.space !== space.id, "settlement returning Home");
  call("do", "t3-tabs.undo");
  await wait(() => call("core", "get", a.terminal, "--json"), t => !t.labels?.settled, "settlement undo");
  call("do", "tab.select", JSON.stringify({ id: a.tab }));

  call("do", "shina-tools.theme");
  await wait(settings, s => s.appearance !== originalAppearance, "theme switching");
  call("do", "shina-tools.theme");
  await wait(settings, s => s.appearance === originalAppearance, "theme restoration");

  if (process.argv.includes("--restart")) {
    const before = call("core", "get", a.terminal, "--json");
    call("do", "app.quit");
    await wait(async () => {
      const r = spawnSync(process.execPath, [runner, "status"], { encoding: "utf8", timeout: 1000 });
      return r.status;
    }, status => status !== 0, "the private window closing");
    call("open");
    const opened = Date.now();
    const restored = await wait(() => debug(a.chat), d => d.connection?.status === "connected" && d.at > opened, "the chat reconnecting after window restart");
    assert.equal(restored.composer.text, "A draft that belongs only to thread A.");
    const after = call("core", "get", a.terminal, "--json");
    assert.equal(after.pid, before.pid, "the same host process survives restarting the window");
  }
  assert.deepEqual(call("status").errors, []);
  console.log("Native workspace smoke passed: live host attachment, isolated persisted drafts, reload, model picker, snooze, settle, undo, and theme." + (process.argv.includes("--restart") ? " Window restart kept the host and draft." : ""));
} finally {
  for (const tab of created.reverse()) call("do", "tab.close", JSON.stringify({ id: tab, confirmed: true }));
  if (originalTab) call("do", "tab.select", JSON.stringify({ id: originalTab }));
  if ((await settings()).appearance !== originalAppearance) call("do", "shina-tools.theme");
  if (!debugWasOn) await unlink(debugFlag);
}
