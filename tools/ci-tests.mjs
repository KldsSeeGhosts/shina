import { execFileSync, spawn } from "node:child_process";
import { appendFileSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { relative, resolve, sep } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";

// CI builds once and transfers the exact executables to separate test runners.
// The default local mode can run both harnesses together on a larger workstation.
export function testSuite(message, manifest = resolve("Cargo.toml")) {
  if (message.reason !== "compiler-artifact" || !message.profile?.test ||
      !message.executable || resolve(message.manifest_path) !== manifest) return null;
  return {
    name: `${message.target.kind.join("+")}/${message.target.name}`,
    group: message.target.kind.includes("test") ? "integration" : "unit",
    executable: message.executable,
  };
}

export function bundlePath(path, workspace = process.cwd()) {
  const name = relative(workspace, path).split(sep).join("/");
  if (!name.startsWith("target/") || name.split("/").includes("..")) {
    throw new Error(`Executable is outside the build tree: ${path}`);
  }
  return name;
}

export function selectSuites(bundle, group, workspace = process.cwd(), commit = process.env.GITHUB_SHA ?? null) {
  if (!["unit", "integration"].includes(group)) throw new Error("Expected unit or integration suite");
  if (bundle.workspace !== resolve(workspace) || bundle.commit !== commit) {
    throw new Error("Test bundle belongs to a different checkout or commit");
  }
  if (!Array.isArray(bundle.suites) || bundle.suites.some(suite =>
    !["unit", "integration"].includes(suite.group) ||
    bundlePath(resolve(workspace, suite.executable), workspace) !== suite.executable)) {
    throw new Error("Invalid test bundle executables");
  }
  const suites = bundle.suites.filter(suite => suite.group === group);
  if (!suites.length) throw new Error(`Test bundle contains no ${group} executables`);
  return suites.map(suite => ({ ...suite, executable: resolve(workspace, suite.executable) }));
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
    const suites = [], applications = [];
    const build = spawn(process.env.CARGO ?? "cargo", [
      "test", "--locked", "--bins", "--tests", "--no-run", "--message-format=json-render-diagnostics",
    ], { stdio: ["ignore", "pipe", "inherit"] });
    createInterface({ input: build.stdout }).on("line", line => {
      try {
        const message = JSON.parse(line);
        const suite = testSuite(message);
        if (suite) suites.push(suite);
        if (message.reason === "compiler-artifact" && !message.profile?.test &&
            message.executable && message.target.kind.includes("bin") &&
            resolve(message.manifest_path) === resolve("Cargo.toml")) {
          applications.push(message.executable);
        }
      } catch {
        console.log(line);
      }
    });
    build.on("error", reject);
    build.on("close", code => code === 0 ? resolveBuild({ suites, applications }) : reject(new Error(`Cargo build exited ${code}`)));
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
  const mode = process.argv[2] ?? "local";
  if (!["build", "run", "local"].includes(mode)) throw new Error("Expected build, run <suite>, or local");
  let suites;
  if (mode === "run") {
    const bundle = JSON.parse(readFileSync("target/ci-tests.json", "utf8"));
    suites = selectSuites(bundle, process.argv[3]);
  } else {
    const built = await buildSuites();
    suites = built.suites;
    if (!suites.length) throw new Error("Cargo produced no test executables");
    if (mode === "build") {
      const bundle = {
        workspace: resolve("."), commit: process.env.GITHUB_SHA ?? null,
        suites: suites.map(suite => ({ ...suite, executable: bundlePath(suite.executable) })),
      };
      // Refuse an incomplete bundle before an apparently green build fans out.
      for (const group of ["unit", "integration"]) selectSuites(bundle, group);
      if (!built.applications.length) throw new Error("Cargo produced no application for the integration fixtures");
      mkdirSync("target", { recursive: true });
      writeFileSync("target/ci-tests.json", JSON.stringify(bundle));
      const files = [...new Set([...suites.map(suite => suite.executable), ...built.applications].map(path => bundlePath(path)))];
      // Tar preserves executable permissions; ZIP artifact extraction does not.
      execFileSync("tar", ["-czf", "target/ci-tests.tar.gz", "target/ci-tests.json", ...files], { stdio: "inherit" });
      console.log(`Test build and bundle: ${((performance.now() - started) / 1000).toFixed(2)}s`);
      return;
    }
  }
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
