import assert from "node:assert/strict";
import { resolve } from "node:path";
import test from "node:test";
import { runSuites, testSuite } from "./ci-tests.mjs";

const artifact = (name, kind = "bin", extra = {}) => ({
  reason: "compiler-artifact", manifest_path: resolve("Cargo.toml"),
  profile: { test: true }, executable: `/target/${name}`,
  target: { name, kind: [kind] }, ...extra,
});

test("select every local test harness, not application or dependency binaries", () => {
  assert.equal(testSuite(artifact("shina")).name, "bin/shina");
  assert.equal(testSuite(artifact("server_runtime", "test")).name, "test/server_runtime");
  assert.equal(testSuite(artifact("future_lib", "lib")).name, "lib/future_lib");
  for (const message of [
    artifact("shina", "bin", { profile: { test: false } }),
    artifact("dependency", "bin", { manifest_path: resolve("vendor/Cargo.toml") }),
    artifact("shina", "bin", { executable: null }), { reason: "build-finished" },
  ]) assert.equal(testSuite(message), null);
});

test("all harnesses start concurrently and the gate waits for both", async () => {
  const started = [], release = [];
  const suites = ["unit", "integration"].map(name => ({ name, executable: name }));
  const pending = runSuites(suites, suite => {
    started.push(suite.name);
    return new Promise(resolveRun => release.push(resolveRun));
  });
  assert.deepEqual(started, ["unit", "integration"]);
  release[0](0);
  let finished = false;
  pending.then(() => { finished = true; });
  await Promise.resolve();
  assert.equal(finished, false);
  release[1](0);
  assert.deepEqual((await pending).failures, []);
});

test("failures, crashes and spawn errors fail the gate without hiding the other suite", async () => {
  for (const outcome of [101, "signal SIGKILL", new Error("spawn failed")]) {
    const seen = [];
    const { failures, results } = await runSuites([
      { name: "unit", executable: "unit" }, { name: "integration", executable: "integration" },
    ], async suite => {
      seen.push(suite.name);
      if (suite.name === "unit") {
        if (outcome instanceof Error) throw outcome;
        return outcome;
      }
      return 0;
    });
    assert.equal(failures.length, 1);
    assert.equal(results.length, 2);
    assert.deepEqual(seen, ["unit", "integration"]);
  }
});

test("missing artifacts refuse success and duplicate Cargo artifacts run once", async () => {
  await assert.rejects(runSuites([], async () => 0), /no test executables/);
  let count = 0;
  const suite = { name: "unit", executable: "unit" };
  const { failures } = await runSuites([suite, suite], async () => { count++; return 0; });
  assert.equal(count, 1);
  assert.deepEqual(failures, []);
});
