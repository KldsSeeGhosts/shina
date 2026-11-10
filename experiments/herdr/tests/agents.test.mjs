import test from "node:test";
import assert from "node:assert/strict";
import { defaultForSpace, updateSpaceDefault, validAgent, draftKey, mayReplaceEmptyThread } from "../workspace/shared/agent-policy.js";

test("space defaults are independent and do not mutate the preference object", () => {
  const before = { studio: "pi", server: "codex" };
  const after = updateSpaceDefault(before, "studio", "claude");
  assert.equal(defaultForSpace("studio", after), "claude");
  assert.equal(defaultForSpace("server", after), "codex");
  assert.equal(before.studio, "pi");
  assert.equal(defaultForSpace("new-space", after), "pi");
});
test("invalid persisted agents fall back to Pi and invalid choices are rejected", () => {
  assert.equal(defaultForSpace("x", { x: "invalid" }), "pi");
  assert.equal(defaultForSpace("x", null), "pi");
  assert.equal(validAgent("claude"), true);
  assert.equal(validAgent("claude-code"), false);
  assert.throws(() => updateSpaceDefault({}, "x", "invalid"), /valid coding agent/);
  assert.throws(() => updateSpaceDefault({}, "", "pi"), /valid coding agent/);
});
test("only an idle, pristine, unsplit thread may be replaced", () => {
  const blank = { empty: true, busy: false, subagent: false, paneCount: 1, hostCount: 1, pending: false };
  assert.equal(mayReplaceEmptyThread(blank), true);
  for (const changes of [{ empty: false }, { busy: true }, { subagent: true }, { pending: true }, { paneCount: 2 }, { hostCount: 2 }, { foreignTerminal: true }]) {
    assert.equal(mayReplaceEmptyThread({ ...blank, ...changes }), false);
  }
});
test("draft fingerprints include text and attachments", () => {
  assert.notEqual(draftKey({ text: "before" }), draftKey({ text: "after" }));
  assert.notEqual(draftKey({ text: "same", chips: [] }), draftKey({ text: "same", chips: [{ path: "/local/image.png" }] }));
  assert.equal(draftKey({ text: "" }), draftKey(null));
});
