import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, mkdir, writeFile, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { probePi } from "../pi-probe.mjs";

const fixture = join(dirname(fileURLToPath(import.meta.url)), "fake-pi.mjs");

async function setup(t, scenario = "success", enrolled = true) {
  const cwd = await mkdtemp(join(tmpdir(), "shina-herdr-probe-"));
  t.after(() => rm(cwd, { recursive: true, force: true }));
  const agentDirectory = join(cwd, "agent");
  await mkdir(agentDirectory);
  if (enrolled) await writeFile(join(agentDirectory, "models.json"), JSON.stringify({
    providers: { cpa: { baseUrl: "http://example.invalid/v1", models: [{ id: "test-model" }] } },
  }));
  await writeFile(join(cwd, "scenario.json"), JSON.stringify({ scenario }));
  return { cwd, agentDirectory, pi: fixture };
}

test("reads only model metadata without a prompt, session, or direct-provider fallback", async (t) => {
  const options = await setup(t);
  const result = await probePi(options);
  assert.deepEqual(result, {
    provider: "cpa", model: "Test 模型", modelId: "test-model",
    thinking: "high", availableModels: 1, promptSent: false,
  });
  const requests = (await readFile(join(options.cwd, "requests.jsonl"), "utf8"))
    .trim().split("\n").map(JSON.parse);
  assert.deepEqual(requests.map((request) => request.type), ["get_state", "get_available_models"]);
  for (const flag of ["--offline", "--no-session", "--no-extensions", "--no-mcp"]) {
    assert.ok(requests.every((request) => request.argv.includes(flag)), flag);
  }
});

test("does not start Pi without CPA enrollment", async (t) => {
  const options = await setup(t, "success", false);
  await assert.rejects(probePi(options), /not enrolled with CPA/);
  await assert.rejects(readFile(join(options.cwd, "requests.jsonl")), { code: "ENOENT" });
});

test("rejects a selected direct provider without changing it", async (t) => {
  await assert.rejects(probePi(await setup(t, "direct")), /will not change it or fall back/);
});

test("reports authentication denial without exposing provider error details", async (t) => {
  await assert.rejects(probePi(await setup(t, "auth")), (error) =>
    error.message.includes("authentication failed")
    && error.message.includes("manually")
    && !error.message.includes("secret-that-must-not-leak"));
});

test("bounds a stalled subprocess and handles an early exit", async (t) => {
  await assert.rejects(probePi({ ...await setup(t, "hang"), timeout: 150 }), /timed out/);
  await assert.rejects(probePi(await setup(t, "exit")), /exited before/);
});
