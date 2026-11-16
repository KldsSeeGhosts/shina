import { spawn } from "node:child_process";
import { appendFileSync } from "node:fs";
import { resolve } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";

// Build every test target once, then run independent harnesses concurrently.
// No filters or changes to libtest's ignore policy.
export function testSuite(message, manifest = resolve("Cargo.toml")) {
  if (message.reason !== "compiler-artifact" || !message.profile?.test ||
      !message.executable || resolve(message.manifest_path) !== manifest) return null;
  return { name: `${message.target.kind.join("+")}/${message.target.name}`, executable: message.executable };
}

export async function runSuites(suites, run) {
  if (!suites.length) throw new Error("Cargo produced no test executables");
  const unique = [...new Map(suites.map(suite => [suite.executable, suite])).values()];
  const results = await Promise.allSettled(unique.map(async suite => {
    const started = performance.now();
    const code = await run(suite);
    return { name: suite.name, code, seconds: (performance.now() - started) / 1000 };
  }));
  const failures = results.flatMap((result, index) =>
    result.status === "rejected" ? [`${unique[index].name}: ${result.reason}`] :
      result.value.code !== 0 ? [`${result.value.name}: exit ${result.value.code}`] : []);
  return { results, failures };
}

function buildSuites() {
  return new Promise((resolveBuild, reject) => {
    const suites = [];
    const build = spawn(process.env.CARGO ?? "cargo", [
      "test", "--locked", "--bins", "--tests", "--no-run", "--message-format=json-render-diagnostics",
    ], { stdio: ["ignore", "pipe", "inherit"] });
    createInterface({ input: build.stdout }).on("line", line => {
      try {
        const suite = testSuite(JSON.parse(line));
        if (suite) suites.push(suite);
      } catch {
        console.log(line);
      }
    });
    build.on("error", reject);
    build.on("close", code => code === 0 ? resolveBuild(suites) : reject(new Error(`Cargo build exited ${code}`)));
  });
}

function runSuite(suite) {
  return new Promise((resolveRun, reject) => {
    const child = spawn(suite.executable, ["--quiet"], { stdio: ["ignore", "pipe", "pipe"] });
    for (const stream of [child.stdout, child.stderr]) {
      createInterface({ input: stream }).on("line", line => console.log(`[${suite.name}] ${line}`));
    }
    child.on("error", reject);
    child.on("close", (code, signal) => resolveRun(code ?? `signal ${signal}`));
  });
}

async function main() {
  const started = performance.now();
  const suites = await buildSuites();
  const buildSeconds = (performance.now() - started) / 1000;
  const { results, failures } = await runSuites(suites, runSuite);
  const lines = [`Test build: ${buildSeconds.toFixed(2)}s`];
  for (const result of results) {
    if (result.status === "fulfilled") {
      lines.push(`${result.value.name}: ${result.value.seconds.toFixed(2)}s, exit ${result.value.code}`);
    } else {
      lines.push(`Test harness error: ${result.reason}`);
    }
  }
  lines.push(`Build and tests total: ${((performance.now() - started) / 1000).toFixed(2)}s`);
  console.log(lines.join("\n"));
  if (process.env.GITHUB_STEP_SUMMARY) {
    appendFileSync(process.env.GITHUB_STEP_SUMMARY, `\n${lines.join("\n\n")}\n`);
  }
  if (failures.length) throw new Error(failures.join("\n"));
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch(error => { console.error(error.message); process.exitCode = 1; });
}
