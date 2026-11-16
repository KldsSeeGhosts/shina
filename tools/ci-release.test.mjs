import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import {
  ASSETS, buildVersion, checksumManifest, parseVersion, prepareVersions, publicationDecision, uploadedChecksums, releaseForTag,
} from "./ci-release.mjs";

const manifest = '[package]\nname = "shina"\nversion = "0.11.0"\n\n[dependencies]\nother = "1"\n';
const lock = 'version = 4\n\n[[package]]\nname = "other"\nversion = "2.0.0"\n\n[[package]]\nname = "shina"\nversion = "0.11.0"\ndependencies = ["other"]\n';

test("release lookup includes drafts before GitHub exposes a published tag", () => {
  const draft = { tag_name: "v0.11.6", draft: true, assets: [{ name: ASSETS[1] }] };
  const read = (path) => {
    assert.equal(path, "repos/owner/shina/releases?per_page=100");
    return [draft, { tag_name: "v0.10.0", draft: false }];
  };
  assert.equal(releaseForTag("owner/shina", "v0.11.6", read), draft);
  assert.equal(releaseForTag("owner/shina", "v0.11.7", read), null);
});

test("versions increase numerically and fit Windows version resources", () => {
  assert.equal(buildVersion(manifest, "42"), "0.11.42");
  assert.deepEqual(parseVersion("v0.11.42"), [0, 11, 42]);
  for (const number of ["0", "-1", "1.2", "01", "65536", "1\nx=y"]) {
    assert.throws(() => buildVersion(manifest, number));
  }
  assert.throws(() => buildVersion(manifest.replace("0.11.0", "0.11.1"), 42));
  assert.throws(() => parseVersion("0.11.42-beta"));
});

test("CI prepares only Shina's package versions and leaves dependencies locked", () => {
  for (const newline of ["\n", "\r\n"]) {
    const result = prepareVersions(manifest.replaceAll("\n", newline), lock.replaceAll("\n", newline), "0.11.42");
    assert.equal(result.manifest, manifest.replace("0.11.0", "0.11.42").replaceAll("\n", newline));
    assert.equal(result.lock, lock.replace("0.11.0", "0.11.42").replaceAll("\n", newline));
  }
  assert.throws(() => prepareVersions(manifest, lock.replace("0.11.0", "0.9.0"), "0.11.42"));
  assert.throws(() => prepareVersions(manifest, lock + lock, "0.11.42"));
  assert.throws(() => prepareVersions(manifest, lock, "invalid"));
});

test("only the current main commit can publish or append its completed platforms", () => {
  const input = { sha: "current", head: "current", version: "0.11.42", latest: { tag_name: "v0.10.0" } };
  assert.equal(publicationDecision(input), "publish");
  assert.equal(publicationDecision({ ...input, head: "newer" }), "superseded");
  assert.equal(publicationDecision({ ...input, latest: { tag_name: "v0.11.43" } }), "superseded");
  assert.equal(publicationDecision({ ...input, latest: { tag_name: "v0.11.42" } }), "superseded");
  assert.equal(publicationDecision({ ...input, latest: null }), "publish");
  assert.equal(publicationDecision({ ...input, existing: { target_commitish: "current", draft: true } }), "publish");
  assert.equal(publicationDecision({ ...input, existing: { target_commitish: "current", draft: false } }), "append");
  assert.equal(publicationDecision({ ...input, latest: { tag_name: "v0.11.42" }, existing: { target_commitish: "current", draft: false } }), "append");
  assert.throws(() => publicationDecision({ ...input, existing: { target_commitish: "other", draft: true } }));
  assert.throws(() => publicationDecision({ ...input, latest: { tag_name: "not-a-version" } }));
});

test("a finished platform can publish alone and checksums grow as other assets arrive", () => {
  const directory = mkdtempSync(join(tmpdir(), "shina-ci-platform-"));
  try {
    writeFileSync(join(directory, ASSETS[0]), "windows");
    const hash = createHash("sha256").update("windows").digest("hex");
    assert.equal(checksumManifest(directory, [ASSETS[0]]), `${hash}  ${ASSETS[0]}\n`);
    const uploaded = [{ name: ASSETS[0], digest: `sha256:${hash}` }];
    assert.equal(uploadedChecksums(uploaded), `${hash}  ${ASSETS[0]}\n`);
    uploaded.push({ name: ASSETS[1], digest: `sha256:${"a".repeat(64)}` });
    assert.match(uploadedChecksums(uploaded), /Shina-macos-aarch64.zip/);
    assert.match(uploadedChecksums(uploaded), /shina.exe/);
    assert.throws(() => uploadedChecksums([]));
    assert.throws(() => uploadedChecksums([{ name: ASSETS[0], digest: null }]));
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});

test("checksums require all architectures and reject unexpected, empty or linked assets", () => {
  const directory = mkdtempSync(join(tmpdir(), "shina-ci-release-"));
  try {
    assert.throws(() => checksumManifest(directory));
    for (const name of ASSETS) writeFileSync(join(directory, name), name);
    const expected = ASSETS.map((name) =>
      `${createHash("sha256").update(name).digest("hex")}  ${name}\n`).join("");
    assert.equal(checksumManifest(directory), expected);
    writeFileSync(join(directory, "SHA256SUMS"), "old checksums");
    assert.equal(checksumManifest(directory), expected);
    writeFileSync(join(directory, "unexpected.txt"), "no");
    assert.throws(() => checksumManifest(directory));
    rmSync(join(directory, "unexpected.txt"));
    writeFileSync(join(directory, ASSETS[0]), "");
    assert.throws(() => checksumManifest(directory));
    rmSync(join(directory, ASSETS[0]));
    if (process.platform !== "win32") {
      symlinkSync(join(directory, ASSETS[1]), join(directory, ASSETS[0]));
      assert.throws(() => checksumManifest(directory));
    }
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});

test("parallel checks and builds cannot publish until every required gate passes", () => {
  const updates = readFileSync(new URL("../.github/workflows/updates.yml", import.meta.url), "utf8");
  const platform = readFileSync(new URL("../.github/workflows/platform-update.yml", import.meta.url), "utf8");
  const publish = readFileSync(new URL("../.github/workflows/publish-update.yml", import.meta.url), "utf8");

  assert.match(updates, /\n  core:\n    name: Core gate\n    needs: \[checks, rust\]\n    if: always\(\)/);
  assert.match(updates, /test "\$CHECKS_RESULT" = success && test "\$RUST_RESULT" = success/);
  assert.match(updates, /cargo \+"\$RUST_TOOLCHAIN" fmt --check/);
  assert.match(updates, /node --test tools\/\*\.test\.mjs/);
  assert.match(updates, /run: node tools\/ci-tests\.mjs/);
  assert.match(updates, /pull_request:\n    branches: \[main\]/);
  assert.match(updates, /group: shina-updates-\$\{\{ github\.event\.pull_request\.number \|\| 'main' \}\}/);
  assert.match(updates, /Preserve normalized mobile fixture failures\n        if: failure\(\)[\s\S]*?path: target\/tmp\/mobile-golden\/\*\.json/);
  for (const platformName of ["windows", "macos", "linux"]) {
    assert.match(updates, new RegExp(`\\n  ${platformName}:\\n    if: github.event_name != 'pull_request'\\n`));
    assert.match(updates, new RegExp(`\\n  publish-${platformName}:\\n    needs: \\[core, ${platformName}\\]`));
    assert.match(updates, new RegExp(`version: \\$\\{\\{ needs\\.${platformName}\\.outputs\\.version \\}\\}`));
  }
  // Build jobs start independently, and each publish waits only for its own
  // platform and the shared gate. A failed Linux build cannot fence Windows.
  for (const name of ["checks", "rust", "windows", "macos", "linux"]) {
    const job = updates.split(`\n  ${name}:\n`)[1].split(/\n  [\w-]+:\n/)[0];
    assert.doesNotMatch(job, /\n    needs:/);
  }
  assert.equal((updates.match(/actions: read/g) ?? []).length, 3);
  assert.match(platform, /value: \$\{\{ jobs\.build\.outputs\.version \}\}/);
  assert.doesNotMatch(platform, /\n  publish:/);
  assert.match(publish, /permissions:\n      actions: read\n      contents: write/);
  assert.match(publish, /group: shina-release-\$\{\{ github\.run_id \}\}/);
  assert.match(publish, /cancel-in-progress: false\n      queue: max/);
  assert.doesNotMatch(publish, /sleep |while true|gh api/);
  assert.match(publish, /node tools\/ci-release\.mjs publish "\$BUILD_VERSION" "\$BUILD_PLATFORM"/);
});

test("core cache survives failures without PR snapshots evicting main, and reruns reuse commit keys", () => {
  const updates = readFileSync(new URL("../.github/workflows/updates.yml", import.meta.url), "utf8");
  const platform = readFileSync(new URL("../.github/workflows/platform-update.yml", import.meta.url), "utf8");
  assert.match(updates, /actions\/cache\/restore@[\da-f]+\s+#.*\n        id: core-cache/);
  assert.match(updates, /Save compiled core dependencies even when a test fails\n        if: \$\{\{ !cancelled\(\) && github\.ref == 'refs\/heads\/main' && steps\.core-cache\.outcome == 'success' && steps\.core-cache\.outputs\.cache-hit != 'true' \}\}/);
  assert.match(updates, /actions\/cache\/save@[\da-f]+/);
  assert.match(updates, /key: \$\{\{ steps\.core-cache\.outputs\.cache-primary-key \}\}/);
  for (const workflow of [updates, platform]) {
    assert.match(workflow, /key: (?:core|fast)-v1-.*\$\{\{ github\.sha \}\}/);
    assert.doesNotMatch(workflow, /key: (?:core|fast)-v1-.*github\.run_id/);
  }
});
