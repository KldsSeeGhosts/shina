import { spawn } from "node:child_process";
import { access, readFile } from "node:fs/promises";
import { homedir } from "node:os";
import { join, extname, resolve, delimiter } from "node:path";
import { fileURLToPath } from "node:url";

async function exists(path) {
  try { await access(path); return true; } catch { return false; }
}

async function enrollment(directory) {
  const extension = join(directory, "extensions/cpa-provider.ts");
  if (await exists(extension)) return extension;
  let registry;
  try { registry = JSON.parse(await readFile(join(directory, "models.json"), "utf8")); } catch {}
  const provider = registry?.providers?.cpa;
  if (provider?.baseUrl?.trim() && provider.models?.some((model) => model.id?.trim())) return null;
  throw new Error("Pi is not enrolled with CPA. Configure Pi's existing CPA provider manually. No settings were changed.");
}

async function findPi() {
  const override = process.env.SHINA_PI_CLI || process.env.STILL_PI_CLI;
  if (override) return override;
  const names = process.platform === "win32" ? ["pi.exe"] : ["pi"];
  for (const dir of (process.env.PATH || "").split(delimiter)) {
    for (const name of names) {
      const candidate = join(dir, name);
      if (await exists(candidate)) return candidate;
    }
  }
  throw new Error("Pi was not found. Set SHINA_PI_CLI to its CLI JavaScript file or executable.");
}

/** Read Pi's RPC metadata. This function cannot submit a model prompt or write Pi settings. */
export async function probePi({
  cwd = process.cwd(),
  pi = null,
  agentDirectory = process.env.PI_CODING_AGENT_DIR || join(homedir(), ".pi/agent"),
  timeout = 20_000,
} = {}) {
  const extension = await enrollment(agentDirectory);
  const executable = pi || await findPi();
  if ([".cmd", ".ps1"].includes(extname(executable))) {
    throw new Error("Use Pi's JavaScript CLI or native executable, not a shell wrapper.");
  }
  const args = [
    "--mode", "rpc", "--no-extensions", "--no-mcp", "--no-skills",
    "--no-prompt-templates", "--no-session", "--offline",
  ];
  if (extension) args.push("--extension", extension);
  const env = { ...process.env };
  delete env.SHINA_MCP_URL;
  delete env.SHINA_MCP_TOKEN;
  const javascript = [".js", ".mjs", ".cjs"].includes(extname(executable));
  const child = spawn(javascript ? process.execPath : executable, javascript ? [executable, ...args] : args, {
    cwd, env, stdio: ["pipe", "pipe", "pipe"], windowsHide: true,
  });
  return new Promise((resolve, reject) => {
    let state = null;
    let models = null;
    let output = "";
    let bytes = 0;
    let settled = false;
    let closed = false;
    let failure;
    let kill;
    const timer = setTimeout(() => finish(new Error("Pi metadata discovery timed out. No prompt was sent.")), timeout);
    function complete() {
      if (failure) return reject(failure);
      resolve({
        provider: "cpa",
        model: state.model.name || state.model.id,
        modelId: state.model.id,
        thinking: state.thinkingLevel || "default",
        availableModels: models.filter((model) => model.provider === "cpa").length,
        promptSent: false,
      });
    }
    function finish(error) {
      if (settled) return;
      settled = true;
      failure = error;
      clearTimeout(timer);
      if (closed) return complete();
      child.stdin.end();
      child.kill("SIGTERM");
      kill = setTimeout(() => child.kill("SIGKILL"), 500);
      kill.unref();
    }
    child.on("error", () => finish(new Error("Pi could not start. Check SHINA_PI_CLI and the project folder.")));
    child.stdin.on("error", () => finish(new Error("Pi closed its RPC input during discovery.")));
    child.once("close", () => {
      // Windows retains the working-directory lock until the process closes. Do not
      // return metadata or errors while the caller still owns a live probe process.
      closed = true;
      clearTimeout(kill);
      if (settled) complete();
      else finish(new Error("Pi exited before returning its model metadata."));
    });
    // Do not log stderr. Provider extensions may put private account details there.
    child.stderr.resume();
    child.stdout.setEncoding("utf8");
    child.stdout.on("data", (chunk) => {
      bytes += Buffer.byteLength(chunk, "utf8");
      if (bytes > 8 * 1024 * 1024) return finish(new Error("Pi's discovery response exceeded the 8 MiB limit."));
      output += chunk;
      let end;
      while ((end = output.indexOf("\n")) >= 0 && !settled) {
        const line = output.slice(0, end);
        output = output.slice(end + 1);
        let record;
        try { record = JSON.parse(line); } catch { continue; }
        if (record.type !== "response") continue;
        if (record.success === false) {
          const authDenied = /unauthori[sz]ed|authentication|api.?key|401|403/i.test(String(record.error));
          return finish(new Error(authDenied
            ? "Pi authentication failed. Repair Pi's existing CPA enrollment manually before retrying. No settings were changed."
            : `Pi rejected ${record.command === "get_state" ? "the state request" : "model discovery"}. No prompt was sent.`));
        }
        if (record.command === "get_state") state = record.data;
        if (record.command === "get_available_models") models = record.data?.models;
        if (state && models) {
          if (state.model?.provider !== "cpa" || !Array.isArray(models)) {
            return finish(new Error("Pi's selected provider is not CPA. The prototype will not change it or fall back to another provider."));
          }
          if (!models.some((model) => model.provider === "cpa")) {
            return finish(new Error("Pi returned no CPA models. Check the existing enrollment manually."));
          }
          finish();
        }
      }
    });
    // This fixed allowlist is the entire RPC request surface of the probe.
    child.stdin.write(JSON.stringify({ id: "shina-probe-state", type: "get_state" }) + "\n");
    child.stdin.write(JSON.stringify({ id: "shina-probe-models", type: "get_available_models" }) + "\n");
  });
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const index = process.argv.indexOf("--cwd");
  try {
    console.log(JSON.stringify(await probePi({ cwd: index >= 0 ? process.argv[index + 1] : undefined })));
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
