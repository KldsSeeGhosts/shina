import { cp, mkdir, writeFile, access } from "node:fs/promises";
import { spawn, spawnSync } from "node:child_process";
import { homedir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const source = dirname(fileURLToPath(import.meta.url));
const repo = resolve(source, "../..");
const runtime = process.env.SHINA_HERDR_HOME || join(homedir(), "Library/Application Support/ShinaHerdr");
const profile = join(runtime, "profile");
const binary = process.env.SHINA_HERDR_BIN || join(runtime, "target/Herdr.app/Contents/MacOS/Herdr");
const core = process.env.SHINA_HERDR_CORE || join(runtime, "target/Herdr Core.app/Contents/MacOS/herdr-core");
const env = {
  ...process.env,
  PATH: `${join(runtime, "bin")}:${process.env.PATH ?? ""}`,
  HERDR_APP_HOME: profile,
  HERDR_TELEMETRY: "0",
  DO_NOT_TRACK: "1",
  HC_SESSION: "shina-prototype",
  XDG_CONFIG_HOME: join(runtime, "config"),
  XDG_STATE_HOME: join(runtime, "state"),
  XDG_RUNTIME_DIR: `/tmp/shina-herdr-${process.getuid()}`,
};

async function createIfMissing(path, value) {
  try { await access(path); } catch { await writeFile(path, JSON.stringify(value, null, 2) + "\n", { mode: 0o600 }); }
}

async function setup() {
  await access(binary);
  await mkdir(profile, { recursive: true, mode: 0o700 });
  for (const folder of ["layouts", "themes", "mods"]) {
    await mkdir(join(profile, folder), { recursive: true });
    await cp(join(source, folder), join(profile, folder), { recursive: true });
  }
  const assets = join(profile, "mods/shina-native/assets");
  await mkdir(assets, { recursive: true });
  for (const name of ["ember", "juniper", "sable", "pip"]) {
    await cp(join(repo, `assets/crew/${name}/Normal.small.png`), join(assets, `${name}.png`));
  }
  const fonts = join(profile, "mods/shina-native/fonts");
  await mkdir(fonts, { recursive: true });
  for (const file of ["InterVariable.ttf", "Inter-LICENSE.txt", "SourceSerif4Variable.ttf", "SourceSerif4-LICENSE.txt"]) {
    await cp(join(repo, "assets/fonts", file), join(fonts, file));
  }
  await createIfMissing(join(profile, "settings.json"), {
    core, telemetry: false, zoom: 1, appearance: "light",
    ui_font: "Anthropic Sans Variable Text", font: "Menlo", font_size: 13, sounds: { enabled: false },
  });
  await createIfMissing(join(profile, "shared.json"), { "frame.welcomed": "skipped" });
  await createIfMissing(join(profile, "window.json"), { state: "windowed", bounds: [80, 80, 1280, 900] });
  await writeFile(join(profile, "shina-runtime.json"), JSON.stringify({
    node: process.execPath, probe: join(source, "pi-probe.mjs"), workspace: repo,
  }, null, 2) + "\n", { mode: 0o600 });
}

function invoke(args) {
  const result = spawnSync(binary, args, { env, stdio: "inherit", timeout: args[0] === "check" ? 30_000 : 15_000 });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
}

function query(args, timeout = 15_000) {
  const result = spawnSync(binary, args, { env, encoding: "utf8", timeout });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(result.stderr.trim() || result.stdout.trim() || "Herdr did not answer.");
  return JSON.parse(result.stdout);
}

const [command = "open", ...args] = process.argv.slice(2);
if (["setup", "open", "check", "init"].includes(command)) await setup();
if (command === "setup") {
  console.log(profile);
} else if (command === "open") {
  const child = spawn(binary, [], { env, stdio: "ignore", detached: true });
  child.on("error", (error) => { console.error(error.message); process.exitCode = 1; });
  child.unref();
  const deadline = Date.now() + 10_000;
  for (;;) {
    try { query(["status"], 1000); break; } catch (error) {
      if (Date.now() >= deadline) throw new Error(`Herdr did not open within ten seconds: ${error.message}`);
      await new Promise((resolve) => setTimeout(resolve, 200));
    }
  }
  console.log(`Herdr prototype profile: ${profile}`);
} else if (command === "check") {
  invoke(["check", join(profile, "mods/shina-native")]);
} else if (command === "init") {
  invoke(["do", "layout.switch", JSON.stringify({ name: "shina" })]);
  let space = query(["ls"]).spaces.find((space) => ["Shina", "Shina prototype"].includes(space.name) && space.folder === repo);
  if (space) {
    invoke(["do", "space.select", JSON.stringify({ id: space.id })]);
  } else {
    const created = query(["do", "space.new", JSON.stringify({
      name: "Shina prototype", folder: repo, tab: "Shell", argv: ["/bin/zsh", "-f"],
    })]);
    space = { id: created.space, tabs: [] };
  }
  const chat = space.tabs.find((tab) => tab.terminals.length === 0);
  if (chat) invoke(["do", "tab.select", JSON.stringify({ id: chat.id })]);
  else invoke(["do", "tab.new", JSON.stringify({
    space: space.id, name: "New thread", mod: "shina-native",
    props: { role: "chat", threadId: `initial-${Date.now()}` },
  })]);
} else if (command === "core") {
  const result = spawnSync(core, args, { env, stdio: "inherit" });
  process.exitCode = result.status ?? 1;
} else {
  if (command === "shot") await new Promise((resolve) => setTimeout(resolve, 350));
  invoke([command, ...args]);
}
