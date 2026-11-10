import test from "node:test";
import assert from "node:assert/strict";
import { workspaceKeys, workspaceLayout, replaceOnce } from "../workspace/customize.mjs";
import { displayText, submitDraft } from "../workspace/mods/chat/shina.js";
import { captainForMachine } from "../workspace/mods/t3-spaces/shina-crew.js";

test("machine portraits are stable and support explicit assignments", () => {
  assert.equal(captainForMachine(null), "ember");
  assert.equal(captainForMachine("studio"), captainForMachine("studio"));
  assert.equal(captainForMachine("studio", { studio: "sable" }), "sable");
  assert.notEqual(captainForMachine("studio", { studio: "../../private" }), "../../private");
});

test("workspace keys preserve unrelated bindings and scope T3 bindings to Shina", () => {
  const existing = { global: { "cmd-k": "user.command" }, "mod:custom": { escape: "overlay.close" } };
  const keys = workspaceKeys({ "layout:herdr-t3": { "cmd-t": "t3-tabs.new" } }, existing);
  assert.equal(keys.global["cmd-k"], "user.command");
  assert.equal(keys["mod:custom"].escape, "overlay.close");
  assert.equal(keys["layout:shina-workspace"]["cmd-t"], "t3-tabs.new");
  assert.equal(keys["layout:shina-workspace"]["cmd-i"], "new-chat.open");
  assert.equal(keys["layout:herdr-t3"], undefined);
  assert.equal(existing["layout:shina-workspace"], undefined);
});

test("workspace layout preserves T3 structure and adds Shina theme and native launcher", () => {
  const original = { children: [{ id: "sidebar", size: { px: 256 } }, { id: "main", content: { pane: { border: {}, focus: {} } } }] };
  const layout = workspaceLayout(original);
  assert.equal(layout.children[0].size.px, 280);
  assert.equal(layout.children[1].content.pane.background, "canvas");
  assert.equal(layout.floating[0].content.mod, "new-chat");
  assert.equal(layout.theme.dark, "shina-dark");
  assert.equal(original.children[0].size.px, 256);
});

test("patches reject absent or ambiguous upstream text", () => {
  assert.throws(() => replaceOnce("different", "old", "new", "test"), /Upstream changed/);
  assert.throws(() => replaceOnce("old old", "old", "new", "test"), /Upstream changed/);
  assert.equal(replaceOnce("old", "old", "new", "test"), "new");
});

test("presentation removes terminal escape codes without rewriting normal text", () => {
  assert.equal(displayText("before \x1b[31mred\x1b[0m"), "before red");
  assert.equal(displayText("Answer.\n\n\x1b[38;2;144;144;144m✻ Worked for 2s · done 9:20 PM\x1b[0m"), "Answer.\n\n");
  assert.equal(displayText("Worked for 2s"), "Worked for 2s");
  assert.equal(displayText("\x1b]0;window title\x07hello"), "hello");
});

test("failed sends retain the draft", async () => {
  let cleared = false;
  await assert.rejects(submitDraft({ text: "hello", images: [] }, {
    send: async () => { throw new Error("offline"); },
    read: () => ({ text: "hello", images: [] }), clear: () => { cleared = true; },
  }), /offline/);
  assert.equal(cleared, false);
});

test("accepted sends clear only the submitted draft", async () => {
  let cleared = 0;
  const original = { text: "hello", images: ["/local/image.png"] };
  const callbacks = { send: async () => ({ disposition: "started" }), read: () => original, clear: () => { cleared++; } };
  assert.equal((await submitDraft(original, callbacks)).cleared, true);
  assert.equal(cleared, 1);
  const edited = await submitDraft(original, { ...callbacks, read: () => ({ text: "new draft", images: [] }) });
  assert.equal(edited.cleared, false);
  assert.equal(cleared, 1);
});
