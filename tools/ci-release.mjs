import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { appendFileSync, lstatSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const ASSETS = [
  "shina.exe",
  "Shina-macos-aarch64.zip",
  "Shina-linux-x86_64.tar.gz",
];
export const PLATFORM_ASSET = Object.fromEntries(
  ["windows-x86_64", "macos-aarch64", "linux-x86_64"].map((platform, i) => [platform, ASSETS[i]]),
);

export function parseVersion(value) {
  const match = /^v?(\d+)\.(\d+)\.(\d+)$/.exec(value);
  if (!match) throw new Error(`Expected a numeric release version, got ${value}`);
  const parts = match.slice(1).map(Number);
  // Windows VERSIONINFO stores each component in an unsigned 16-bit field.
  if (parts.some((part) => !Number.isInteger(part) || part > 65535)) {
    throw new Error("Version components must be between 0 and 65535");
  }
  return parts;
}

function packageSection(manifest) {
  const section = /^\[package\]\r?\n([\s\S]*?)(?=^\[|(?![\s\S]))/m.exec(manifest)?.[1];
  if (!section || !/^name = "shina"\r?$/m.test(section)) {
    throw new Error("Expected Shina's [package] section");
  }
  return section;
}

function versionField(section) {
  const matches = [...section.matchAll(/^version = "([^"]+)"\r?$/gm)];
  if (matches.length !== 1) throw new Error("Expected exactly one package version");
  return matches[0][1];
}

export function buildVersion(manifest, runNumber) {
  const [major, minor, patch] = parseVersion(versionField(packageSection(manifest)));
  if (patch !== 0) throw new Error("Keep Cargo.toml's patch at zero for automatic builds");
  if (!/^[1-9]\d*$/.test(String(runNumber))) throw new Error("Expected a positive Actions run number");
  const version = `${major}.${minor}.${runNumber}`;
  parseVersion(version);
  return version;
}

export function prepareVersions(manifest, lock, version) {
  parseVersion(version);
  const section = packageSection(manifest);
  const oldVersion = versionField(section);
  const records = lock.split(/(?=^\[\[package\]\]\r?$)/m);
  const indices = records.flatMap((record, index) =>
    /^name = "shina"\r?$/m.test(record) ? [index] : []);
  if (indices.length !== 1 || versionField(records[indices[0]]) !== oldVersion) {
    throw new Error("Cargo.lock must contain exactly one matching Shina package");
  }
  const replace = (text) => text.replace(/^version = "[^"]+"/m, `version = "${version}"`);
  records[indices[0]] = replace(records[indices[0]]);
  return {
    manifest: manifest.replace(section, replace(section)),
    lock: records.join(""),
  };
}

export function checksumManifest(directory, assets = ASSETS) {
  const unexpected = readdirSync(directory).filter((name) =>
    !assets.includes(name) && name !== "SHA256SUMS");
  if (unexpected.length) throw new Error(`Unexpected release files: ${unexpected.join(", ")}`);
  return assets.map((name) => {
    const path = join(directory, name);
    const stat = lstatSync(path);
    if (!stat.isFile() || stat.isSymbolicLink() || stat.size === 0) {
      throw new Error(`Expected a nonempty release file: ${name}`);
    }
    return `${createHash("sha256").update(readFileSync(path)).digest("hex")}  ${name}\n`;
  }).join("");
}

export function uploadedChecksums(assets) {
  const available = assets.filter((asset) => ASSETS.includes(asset.name));
  if (!available.length) throw new Error("No platform assets have uploaded");
  return available.sort((a, b) => a.name.localeCompare(b.name)).map((asset) => {
    if (!/^sha256:[a-f0-9]{64}$/i.test(asset.digest ?? "")) {
      throw new Error(`GitHub has no checksum for ${asset.name}`);
    }
    return `${asset.digest.slice(7).toLowerCase()}  ${asset.name}\n`;
  }).join("");
}

function compareVersions(a, b) {
  const left = parseVersion(a), right = parseVersion(b);
  for (let i = 0; i < 3; i++) {
    if (left[i] !== right[i]) return Math.sign(left[i] - right[i]);
  }
  return 0;
}

export function publicationDecision({ sha, head, version, latest, existing }) {
  parseVersion(version);
  if (head !== sha) return "superseded";
  if (latest && compareVersions(version, latest.tag_name) < 0) return "superseded";
  if (existing) {
    if (existing.target_commitish !== sha) throw new Error("Release tag belongs to another commit");
    return existing.draft ? "publish" : "append";
  }
  if (latest && compareVersions(version, latest.tag_name) <= 0) return "superseded";
  return "publish";
}

function gh(args) {
  return execFileSync("gh", args, { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim();
}

function api(path, optional = false) {
  try {
    return JSON.parse(gh(["api", path]));
  } catch (error) {
    // A missing release is expected. Authentication and transport failures are not.
    if (optional && String(error.stderr).includes("(HTTP 404)")) return null;
    throw error;
  }
}

function publish(directory, version, platform) {
  const { GITHUB_REPOSITORY: repo, GITHUB_SHA: sha, GITHUB_REF: ref, GITHUB_EVENT_NAME: event } = process.env;
  if (ref !== "refs/heads/main" || !["push", "workflow_dispatch"].includes(event)) {
    throw new Error("Only main pushes or manual runs on main may publish updates");
  }
  if (!/^[\w.-]+\/[\w.-]+$/.test(repo ?? "") || !/^[a-f0-9]{40}$/.test(sha ?? "")) {
    throw new Error("Expected an exact Actions repository and commit");
  }
  const tag = `v${version}`;
  const assetName = PLATFORM_ASSET[platform];
  if (!assetName) throw new Error(`Unknown update platform: ${platform}`);
  // Each platform publishes its own completed build, without waiting for the others.
  checksumManifest(directory, [assetName]);
  const decision = () => publicationDecision({
    sha, version,
    head: api(`repos/${repo}/git/ref/heads/main`).object.sha,
    latest: api(`repos/${repo}/releases/latest`, true),
    existing: api(`repos/${repo}/releases/tags/${tag}`, true),
  });
  let state = decision();
  if (state === "publish" || state === "append") {
    const existing = api(`repos/${repo}/releases/tags/${tag}`, true);
    if (!existing) {
      gh(["release", "create", tag, "--repo", repo, "--draft", "--target", sha,
        "--title", `Shina ${version}`, "--notes",
        `Personal development update from main.\n\nSource commit: ${sha}\n\nWindows, Apple-silicon Mac and portable Linux builds arrive independently. This fast build does not run test or lint gates unless requested manually. Restart from Shina's update button to install on Windows or Mac.`]);
    }
    const uploaded = existing?.assets.find((asset) => asset.name === assetName);
    if (!uploaded) {
      gh(["release", "upload", tag, join(directory, assetName), "--repo", repo]);
    }
    // Publish jobs serialize their writes. The manifest grows as platforms finish.
    const release = api(`repos/${repo}/releases/tags/${tag}`);
    writeFileSync(join(directory, "SHA256SUMS"), uploadedChecksums(release.assets));
    gh(["release", "upload", tag, join(directory, "SHA256SUMS"), "--repo", repo, "--clobber"]);
    state = decision();
    if (state === "publish" || state === "append") {
      gh(["release", "edit", tag, "--repo", repo, "--draft=false", "--latest"]);
      state = "published";
    }
  }
  const message = `${tag}: ${state}, commit ${sha}`;
  console.log(message);
  if (process.env.GITHUB_STEP_SUMMARY) {
    appendFileSync(process.env.GITHUB_STEP_SUMMARY,
      `${message}\n\nhttps://github.com/${repo}/releases/tag/${tag}\n`);
  }
}

function main() {
  const [command, argument, platform] = process.argv.slice(2);
  if (command === "plan") {
    const version = buildVersion(readFileSync("Cargo.toml", "utf8"), argument);
    console.log(version);
    if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, `version=${version}\n`);
  } else if (command === "prepare") {
    const result = prepareVersions(readFileSync("Cargo.toml", "utf8"), readFileSync("Cargo.lock", "utf8"), argument);
    writeFileSync("Cargo.toml", result.manifest);
    writeFileSync("Cargo.lock", result.lock);
  } else if (command === "publish") {
    publish(resolve("target/release-assets"), argument, platform);
  } else {
    throw new Error("Usage: node tools/ci-release.mjs plan <run-number> | prepare <version> | publish <version> <platform>");
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();
