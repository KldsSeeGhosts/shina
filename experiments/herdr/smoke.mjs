// Runs only against the separate prototype profile. No model request is sent.
import { spawnSync } from "node:child_process";
import { readFile } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import assert from "node:assert/strict";
import { randomUUID } from "node:crypto";

const runner = fileURLToPath(new URL("run.mjs", import.meta.url));
const runtime = process.env.SHINA_HERDR_HOME || join(homedir(), "Library/Application Support/ShinaHerdr");
const profile = join(runtime, "profile");
function call(...args) {
  const result = spawnSync(process.execPath, [runner, ...args], { encoding: "utf8", timeout: 35_000 });
  assert.equal(result.status, 0, result.stderr || result.stdout || String(result.error));
  return args[0] === "check" ? null : JSON.parse(result.stdout);
}
async function waitFor(read, matches) {
  const until = Date.now() + 5000;
  while (Date.now() < until) {
    const value = await read();
    if (matches(value)) return value;
    await new Promise((resolve) => setTimeout(resolve, 75));
  }
  throw new Error("The native prototype did not reach the expected state within five seconds.");
}
const settings = () => readFile(join(profile, "settings.json"), "utf8").then(JSON.parse);
const snapshot = async (tab) => {
  const before = JSON.parse(await readFile(join(profile, "shared.json"), "utf8"))["shina.snapshot"]?.snapshotId;
  return waitFor(
    async () => {
      call("do", "shina-native.snapshot");
      return JSON.parse(await readFile(join(profile, "shared.json"), "utf8"))["shina.snapshot"];
    },
    (value) => value?.tab === tab && value.snapshotId !== before,
  );
};

const before = call("ls");
const space = before.spaces.find((space) => ["Shina", "Shina prototype"].includes(space.name));
assert.ok(space, "Run the prototype's open and init commands first.");
const originalTab = space.tabs.find((tab) => tab.active)?.id;
const originalAppearance = (await settings()).appearance;
const created = [];
try {
  const threadA = `smoke-${randomUUID()}`;
  const a = call("do", "tab.new", JSON.stringify({
    space: space.id, name: "Draft isolation check", mod: "shina-native",
    props: { role: "chat", captain: "juniper", threadId: threadA },
  }));
  created.push(a.tab);
  await snapshot(a.tab);
  call("do", "shina-native.focus");
  assert.ok((await snapshot(a.tab)).focused, "the native textarea receives keyboard focus");
  call("do", "shina-native.draft-demo");
  const first = await snapshot(a.tab);
  assert.equal(first.thread, threadA);
  assert.equal(first.draft, "A draft kept while switching native Herdr tabs.");
  assert.equal(first.messages.length, 0);

  const threadB = `smoke-${randomUUID()}`;
  const b = call("do", "tab.new", JSON.stringify({
    space: space.id, name: "Composer interaction check", mod: "shina-native",
    props: { role: "chat", captain: "sable", threadId: threadB },
  }));
  created.push(b.tab);
  await snapshot(b.tab);
  call("do", "shina-native.submit-demo");
  const second = await snapshot(b.tab);
  assert.equal(second.thread, threadB);
  assert.equal(second.messages.length, 2);
  assert.match(second.messages[1].text, /local prototype response/);
  assert.equal(second.draft, "");

  // Reload both copies while B has focus. A must retain A's identity and draft.
  call("check");
  await new Promise((resolve) => setTimeout(resolve, 500));
  call("do", "tab.select", JSON.stringify({ id: a.tab }));
  const restored = await snapshot(a.tab);
  assert.equal(restored.thread, threadA);
  assert.equal(restored.messages.length, 0);
  assert.equal(restored.draft, first.draft);
  call("do", "tab.settle", JSON.stringify({ id: b.tab }));
  assert.ok(call("ls").spaces.flatMap((space) => space.tabs).find((tab) => tab.id === b.tab)?.settled);

  call("do", "shina-native.theme");
  await waitFor(settings, (value) => value.appearance !== originalAppearance);
  call("do", "shina-native.theme");
  await waitFor(settings, (value) => value.appearance === originalAppearance);
  assert.deepEqual(call("status").errors, []);
  console.log("Native smoke check passed: textarea focus, composer submit, isolated drafts, reload, tab settlement, and theme switching.");
} finally {
  // These exact tabs were created by this test. Leave existing prototype work untouched.
  for (const id of created.reverse()) call("do", "tab.close", JSON.stringify({ id, confirmed: true }));
  if (originalTab) call("do", "tab.select", JSON.stringify({ id: originalTab }));
}
