import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdtempSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
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
