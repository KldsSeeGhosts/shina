import { access, cp, mkdir, mkdtemp, readFile, readdir, writeFile, symlink, rm } from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { homedir, hostname } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { customize, workspaceLayout, workspaceKeys, replaceOnce } from "./workspace/customize.mjs";

const source = dirname(fileURLToPath(import.meta.url));
const runtime = process.env.SHINA_HERDR_HOME || join(homedir(), "Library/Application Support/ShinaHerdr");
const profile = join(runtime, "profile");
const binary = process.env.SHINA_HERDR_BIN || join(runtime, "target/Herdr.app/Contents/MacOS/Herdr");
const core = process.env.SHINA_HERDR_CORE || join(runtime, "target/Herdr Core.app/Contents/MacOS/herdr-core");
const env = {
  ...process.env, HERDR_APP_HOME: profile, HERDR_TELEMETRY: "0", DO_NOT_TRACK: "1",
  PATH: `${join(runtime, "bin")}:${process.env.PATH ?? ""}`,
  HC_SESSION: "shina-prototype", XDG_CONFIG_HOME: join(runtime, "config"),
  XDG_STATE_HOME: join(runtime, "state"), XDG_RUNTIME_DIR: `/tmp/shina-herdr-${process.getuid()}`,
};
const pins = JSON.parse(await readFile(join(source, "workspace/upstreams.json"), "utf8"));
const exists = async (file) => { try { await access(file); return true; } catch { return false; } };
function exec(program, args, options = {}) {
  const result = spawnSync(program, args, { env, encoding: "utf8", timeout: 30_000, ...options });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${program}: ${result.stderr || result.stdout || result.status}`);
  return result.stdout.trim();
}
async function json(file, fallback = {}) {
  return await exists(file) ? JSON.parse(await readFile(file, "utf8")) : fallback;
}
async function output(file, value) {
  await mkdir(dirname(file), { recursive: true });
  await writeFile(file, JSON.stringify(value, null, 2) + "\n", { mode: 0o600 });
}

const [command = "install"] = process.argv.slice(2);
if (!["install", "check", "open"].includes(command)) throw new Error("Use install, check, or open.");
await access(binary);
await access(core);
const trees = {};
for (const [name, pin] of Object.entries(pins)) {
  const tree = join(runtime, "upstream", name);
  if (!await exists(join(tree, ".git"))) {
    if (command !== "install") throw new Error("Run workspace.mjs install first.");
    exec("git", ["clone", pin.url, tree]);
    exec("git", ["-C", tree, "checkout", "--detach", pin.commit]);
  }
  const actual = exec("git", ["-C", tree, "rev-parse", "HEAD"]);
  if (actual !== pin.commit || exec("git", ["-C", tree, "status", "--porcelain"])) {
    throw new Error(`Keep ${tree} at clean pinned commit ${pin.commit}; customization belongs in workspace/.`);
  }
  trees[name] = tree;
}

if (command === "install") {
  await mkdir(join(runtime, "bin"), { recursive: true });
  if (!await exists(join(runtime, "bin/hc"))) await symlink(core, join(runtime, "bin/hc"));
  const chat = trees["herdr-chat"], t3 = trees["herdr-t3"];
  if (!await exists(join(chat, "node_modules/highlight.js"))) {
    exec("npm", ["ci", "--ignore-scripts", "--no-audit", "--no-fund"], { cwd: chat, timeout: 60_000 });
  }
  if (await exists(join(homedir(), "Library/Fonts/AnthropicSerif-Roman-Variable.ttf"))) {
    const python = join(runtime, "font-tools/bin/python");
    if (!await exists(python)) exec("python3", ["-m", "venv", join(runtime, "font-tools")]);
    const fontTools = spawnSync(python, ["-c", "import fontTools; assert fontTools.__version__ == '4.60.1'"], { env, encoding: "utf8", timeout: 5000 });
    if (fontTools.status !== 0) {
      exec(python, ["-m", "pip", "--disable-pip-version-check", "--no-cache-dir", "install", "fonttools==4.60.1"], { timeout: 60_000 });
    }
    exec(python, [join(source, "workspace/prepare-fonts.py"), "--source", join(homedir(), "Library/Fonts"), "--output", join(profile, "fonts")],
    { timeout: 60_000 });
  }
  const fonts = await exists(join(profile, "fonts/ShinaSerif-Regular.ttf"))
    ? { sans: "Shina Sans", serif: "Shina Serif" }
    : { sans: "Anthropic Sans Variable Text", serif: "Anthropic Serif Variable Text" };
  // Do not run either upstream installer. They can modify global instructions,
  // Pi extensions, and other checkouts. All generated files stay in this profile.
  const stage = await mkdtemp(join(runtime, "workspace-stage-"));
  const names = (await readdir(join(t3, "mods"))).filter(name => name.startsWith("t3-"));
  for (const name of names) await cp(join(t3, "mods", name), join(stage, name), { recursive: true });
  for (const name of ["chat", "new-chat"]) {
    names.push(name);
    await cp(join(chat, "herdr-app/mods", name), join(stage, name), { recursive: true });
  }
  for (const name of ["shina-home", "shina-tools", "shina-agent"]) {
    names.push(name);
    await cp(join(source, "workspace/mods", name), join(stage, name), { recursive: true });
    for (const file of ["focus.js", "tip.js", "sidebar.js"]) {
      await cp(join(t3, "shared", file), join(stage, name, file));
    }
  }
  await mkdir(join(stage, "shina-tools/icons"), { recursive: true });
  for (const name of ["folder", "git-branch"]) {
    await cp(join(t3, `mods/t3-inbox/icons/${name}.svg`), join(stage, `shina-tools/icons/${name}.svg`));
  }
  await cp(join(source, "mods/shina-native/assets/theme.svg"), join(stage, "shina-tools/icons/sun.svg"));
  await mkdir(join(stage, "shina-home/assets"), { recursive: true });
  await cp(join(source, "../../assets/crew/ember/Normal.small.png"), join(stage, "shina-home/assets/ember.png"));
  await cp(join(source, "workspace/mods/chat/shina.js"), join(stage, "chat/shina.js"));
  await cp(join(source, "workspace/mods/chat/shina-agent.js"), join(stage, "chat/shina-agent.js"));
  await cp(join(source, "workspace/mods/t3-spaces/shina-crew.js"), join(stage, "t3-spaces/shina-crew.js"));
  await mkdir(join(stage, "t3-spaces/shina-assets"), { recursive: true });
  for (const name of ["ember", "juniper", "sable", "pip"]) {
    await cp(join(source, `../../assets/crew/${name}/Normal.small.png`), join(stage, `t3-spaces/shina-assets/${name}.png`));
  }
  // Mods cannot import outside their folders. Keep one source for agent policy
  // and copy it to every consumer at deployment.
  for (const name of names) {
    for (const file of ["agents.js", "agent-policy.js"]) {
      await cp(join(source, "workspace/shared", file), join(stage, name, file));
    }
  }

  for (const name of names) {
    for (const entry of await readdir(join(stage, name))) {
      if (!entry.endsWith(".js")) continue;
      const file = join(stage, name, entry);
      const before = await readFile(file, "utf8");
      const after = customize(`${name}/${entry}`, before, fonts);
      await writeFile(file, after === before ? after : "// Modified for Shina. Pinned upstream sources and licenses are recorded in shina-workspace-install.json and licenses/.\n" + after);
    }
  }
  const which = name => exec("/usr/bin/which", [name]);
  await output(join(stage, "chat/config.json"), {
    host: join(chat, "bin/herdr-chat-host"), node: process.execPath, hc: core,
    version: (await json(join(chat, "package.json"))).version,
    pi: which("pi"), claude: which("claude"), codex: which("codex"),
  });
  await output(join(stage, "chat/keys.default.json"), {
    ...await json(join(stage, "chat/keys.default.json")), latest: "cmd-end", tui: "cmd-alt-enter",
  });
  // The host's pane watcher also needs the private core's full path and session.
  // Enforce it at process creation, not in the user's provider configuration.
  const connectionFile = join(stage, "chat/connection.js");
  const connection = await readFile(connectionFile, "utf8");
  await writeFile(connectionFile, replaceOnce(connection,
    "if (cfg.node) env.HERDR_CHAT_NODE = cfg.node;",
    `if (cfg.node) env.HERDR_CHAT_NODE = cfg.node;
    env.HERDR_CHAT_HC = cfg.hc;
    env.HERDR_APP_HOME = appHome();
    env.HC_SESSION = "shina-prototype";`, "chat host environment"));
  for (const name of names) {
    const result = exec(binary, ["check", join(stage, name)]);
    console.log(`${name}: ${result || "checked"}`);
  }

  const computer = process.platform === "darwin"
    ? spawnSync("/usr/sbin/scutil", ["--get", "ComputerName"], { encoding: "utf8", timeout: 5000 })
    : null;
  const localName = computer?.status === 0 ? computer.stdout.trim() : hostname().split(".")[0];
  const layout = workspaceLayout(await json(join(t3, "layouts/herdr-t3.json")), fonts, localName);
  const keysFile = join(profile, "keys.json");
  const keys = workspaceKeys(await json(join(t3, "keys.json")), await json(keysFile));
  const backup = join(runtime, "backups", `workspace-${Date.now()}`);
  await mkdir(backup, { recursive: true });
  for (const relative of ["keys.json", "layouts/shina-workspace.json", ...names.map(n => `mods/${n}`)]) {
    if (await exists(join(profile, relative))) {
      await mkdir(dirname(join(backup, relative)), { recursive: true });
      await cp(join(profile, relative), join(backup, relative), { recursive: true });
    }
  }
  for (const name of names) await cp(join(stage, name), join(profile, "mods", name), { recursive: true });
  await output(join(profile, "layouts/shina-workspace.json"), layout);
  await output(keysFile, keys);
  for (const name of ["shina-light.json", "shina-dark.json"]) {
    await cp(join(source, "themes", name), join(profile, "themes", name));
  }
  for (const [name, tree] of Object.entries(trees)) {
    await mkdir(join(profile, "licenses", name), { recursive: true });
    await cp(join(tree, "LICENSE"), join(profile, "licenses", name, "LICENSE"));
  }
  await cp(join(source, "workspace/THIRD_PARTY.md"), join(profile, "licenses/SHINA-NOTICES.md"));
  for (const provider of ["pi", "claude", "codex"]) {
    const file = join(runtime, `config/herdr-core/templates/shina-${provider}-chat.md`);
    const quote = value => "'" + value.replaceAll("'", "'\\''") + "'";
    const launch = ["/usr/bin/env", `HERDR_APP_HOME=${profile}`, `HERDR_CHAT_NODE=${process.execPath}`, `HERDR_CHAT_HC=${core}`,
      join(chat, "bin/herdr-chat-subagent"), "--provider", provider].map(quote).join(" ");
    const body = `---\ndescription: Native ${provider} chat in the private Shina workspace.\ncommand: ${launch}\ninstalled-by: shina-workspace\n---\nStay within the parent agent's assigned task. Do not delegate to another agent.\n`;
    if (await exists(file) && !(await readFile(file, "utf8")).includes("installed-by: shina-workspace")) {
      throw new Error(`Refusing to overwrite a template Shina does not own: ${file}`);
    }
    await mkdir(dirname(file), { recursive: true });
    await writeFile(file, body, { mode: 0o600 });
  }
  await output(join(profile, "shina-workspace-install.json"), { installedAt: new Date().toISOString(), upstreams: pins, backup, mods: names });
  // This is the exact staging directory this invocation created, not user data.
  await rm(stage, { recursive: true });
  console.log(`Installed Shina workspace. Previous files: ${backup}`);
} else if (command === "check") {
  const install = await json(join(profile, "shina-workspace-install.json"));
  if (!install.mods) throw new Error("Run workspace.mjs install first.");
  for (const name of install.mods) console.log(exec(binary, ["check", join(profile, "mods", name)]));
} else {
  console.log(exec(process.execPath, [join(source, "run.mjs"), "open"]));
  const state = JSON.parse(exec(binary, ["status"]));
  if (state.layout !== "shina-workspace") {
    console.log(exec(binary, ["do", "layout.switch", JSON.stringify({ name: "shina-workspace" })]));
    console.log(exec(binary, ["do", "t3-tabs.home"]));
  }
}
