import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import { arch, platform, release } from "node:os";
import { resolve } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";

// These formats are from the pinned Rune revision's frame/profile.rs,
// frame/live_log.rs and tooling/trace.rs. Phase times overlap. Never add them.
const nativePhases = [
  "input", "wake", "prebuild", "acquire", "build", "view", "commit", "layout",
  "text", "paint", "apply", "upload", "encode", "submit", "present", "total", "cpu",
];
const nativeCounts = ["nodes", "shaped", "instances", "inputs", "wakes"];
const headlessPhases = [
  "ms", "cpu_ms", "update_ms", "encode_ms", "view_cpu", "commit_cpu",
  "layout_cpu", "text_cpu", "paint_cpu", "apply_cpu",
];
const own = (value, key) => Object.hasOwn(value, key);
const finite = value => typeof value === "number" && Number.isFinite(value) && value >= 0;

export function distribution(values, unit, missing = 0, budget = null) {
  if (!values.length) return { unit, samples: 0, missing };
  const sorted = [...values].sort((a, b) => a - b);
  const rank = fraction => sorted[Math.max(0, Math.ceil(sorted.length * fraction) - 1)];
  return {
    unit, samples: sorted.length, missing, p50: rank(0.5), p95: rank(0.95),
    p99: rank(0.99), max: sorted.at(-1),
    ...(budget === null ? {} : {
      referenceBudgetMs: budget,
      samplesAboveReferenceBudget: sorted.filter(value => value > budget).length,
    }),
  };
}

export async function summarizeFrames(lines, {
  kind, skip = 1, refreshHz = null, countedAllocations = false,
} = {}) {
  if (!["headless", "native", "trace"].includes(kind)) throw new Error("Choose headless, native or trace");
  if (!Number.isSafeInteger(skip) || skip < 0) throw new Error("skip must be a nonnegative integer");
  if (refreshHz !== null && (!finite(refreshHz) || refreshHz === 0 || refreshHz > 1000)) {
    throw new Error("refreshHz must be greater than zero and at most 1000");
  }
  if (countedAllocations && kind !== "headless") {
    throw new Error("Only the headless log has per-frame allocation counts");
  }
  const metrics = new Map();
  const samples = (name, unit) => {
    if (!metrics.has(name)) metrics.set(name, { values: [], missing: 0, unit });
    return metrics.get(name);
  };
  let lineNumber = 0, frames = 0, measured = 0, header = null, end = null;
  let presented = 0, skippedPresent = 0, unknownPresent = 0;
  const add = (record, key, unit, divisor = 1, name = key, count = false) => {
    const sample = samples(name, unit);
    if (!own(record, key) || record[key] === null) {
      sample.missing++;
    } else if (!finite(record[key]) || (count && !Number.isSafeInteger(record[key]))) {
      throw new Error(`Invalid ${key} on line ${lineNumber}`);
    } else {
      sample.values.push(record[key] / divisor);
    }
  };
  for await (const text of lines) {
    lineNumber++;
    if (!text.trim()) continue;
    if (Buffer.byteLength(text) > 16 * 1024 * 1024) throw new Error(`Line ${lineNumber} exceeds 16 MiB`);
    let row;
    try { row = JSON.parse(text); } catch { throw new Error(`Invalid JSON on line ${lineNumber}`); }
    if (!row || typeof row !== "object" || Array.isArray(row)) throw new Error(`Invalid record on line ${lineNumber}`);
    if (row.type === "header") {
      if (kind !== "trace" || header || frames || !["virtual", "wall"].includes(row.clock)) {
        throw new Error(`Unexpected trace header on line ${lineNumber}`);
      }
      header = row;
      continue;
    }
    if (row.end === true) {
      if (kind !== "headless" || end || !Number.isSafeInteger(row.frames) || row.frames !== frames) {
        throw new Error(`Invalid closing record on line ${lineNumber}`);
      }
      end = row;
      continue;
    }
    const valid = kind === "headless" ? row.k === frames && finite(row.update_ms) && finite(row.encode_ms)
      : kind === "native" ? finite(row.t) && finite(row.total) && finite(row.build)
        : header && finite(row.t) && row.n === 1 && row.quality && typeof row.quality === "object";
    if (!valid || end) throw new Error(`Wrong or out-of-order ${kind} record on line ${lineNumber}`);
    if (++frames > 100_000) throw new Error("More than 100,000 frames; split the recording");
    if (frames <= skip) continue;
    measured++;
    if (kind === "native") {
      for (const phase of nativePhases) add(row, phase, "ms", 1e6);
      for (const count of nativeCounts) add(row, count, "count", 1, count, true);
    } else if (kind === "headless") {
      for (const phase of headlessPhases) add(row, phase, "ms");
      for (const count of ["commits", "shaped", "events"]) add(row, count, "count", 1, count, true);
      if (countedAllocations) add(row, "allocations", "allocations", 1, "allocations", true);
    } else {
      // Despite its name, Rune quality.cpu_ms uses Instant::elapsed, a wall clock.
      add(row.quality, "cpu_ms", "ms", 1, "submit_work_wall_ms");
      if (header.clock === "wall") {
        if (!own(row, "present_ms")) unknownPresent++;
        else if (row.present_ms === null) skippedPresent++;
        else if (finite(row.present_ms)) presented++;
        else throw new Error(`Invalid present_ms on line ${lineNumber}`);
        if (own(row, "present_dt_ms") && !finite(row.present_ms)) {
          throw new Error(`Presentation interval without a presentation on line ${lineNumber}`);
        }
        // Rune emits dt only for adjacent continuous presentations. Do not
        // derive it from t, target_ms or gaps across idle/skipped frames.
        add(row, "present_dt_ms", "ms");
      } else if (own(row, "present_ms") || own(row, "present_dt_ms")) {
        throw new Error(`Virtual trace claims native presentation on line ${lineNumber}`);
      }
    }
  }
  if (!measured) throw new Error("No measured frames remain after warmup removal");
  const budget = refreshHz === null ? null : 1000 / refreshHz;
  const warnings = [
    "Input identity does not prove capture completion or that source, fixture and hardware match another run.",
    "Phase times overlap and must not be summed. Durations do not establish an FPS or a speed ratio.",
  ];
  if (kind === "headless" || header?.clock === "virtual") {
    warnings.push("Virtual cadence is not a native refresh-rate measurement.");
  }
  if (kind === "headless" && !countedAllocations) {
    warnings.push("Allocation counts excluded: an active counting allocator was not declared.");
  }
  if (kind === "native") warnings.push("Native phase log has no actual presentation timestamps or per-frame allocations.");
  if (kind === "trace" && header.clock === "wall" && !presented) {
    warnings.push("No actual presentations observed; submission timestamps do not prove display cadence.");
  }
  return {
    format: 1, kind, frames, discardedWarmupFrames: skip, measuredFrames: measured,
    declaredRefreshHz: refreshHz, referenceFrameBudgetMs: budget,
    clock: header?.clock ?? (kind === "headless" ? "virtual" : "wall"),
    allocationCounter: countedAllocations ? "declared-active-not-independently-verified" : "not-verified",
    presentation: kind === "trace" && header.clock === "wall"
      ? { presented, skipped: skippedPresent, unknown: unknownPresent } : null,
    metrics: Object.fromEntries([...metrics].map(([name, sample]) => [
      name, distribution(sample.values, sample.unit, sample.missing,
        sample.unit === "ms" ? budget : null),
    ])),
    closingRecordObserved: !!end,
    warnings,
  };
}

export async function summarizeFile(path, options) {
  const input = createReadStream(path);
  const hash = createHash("sha256");
  input.on("data", chunk => hash.update(chunk));
  const lines = createInterface({ input, crlfDelay: Infinity });
  try {
    const report = await summarizeFrames(lines, options);
    return { ...report, input: { path: resolve(path), sha256: hash.digest("hex") } };
  } finally {
    lines.close();
    input.destroy();
  }
}

export function parseArgs(args) {
  const options = {};
  let path;
  for (let index = 0; index < args.length; index++) {
    const flag = args[index];
    if (flag === "--counted-allocations") { options.countedAllocations = true; continue; }
    if (!["--kind", "--input", "--skip", "--refresh-hz"].includes(flag) || index + 1 === args.length) {
      throw new Error("Use --kind headless|native|trace --input FILE [--skip N] [--refresh-hz N] [--counted-allocations]");
    }
    const value = args[++index];
    if (flag === "--input") path = value;
    else if (flag === "--kind") options.kind = value;
    else if (flag === "--skip") options.skip = Number(value);
    else options.refreshHz = Number(value);
  }
  if (!path) throw new Error("--input is required");
  return { path, options };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const { path, options } = parseArgs(process.argv.slice(2));
    const report = await summarizeFile(path, options);
    report.analysisHost = { platform: platform(), arch: arch(), release: release(), node: process.version };
    console.log(JSON.stringify(report, null, 2));
  } catch (error) {
    console.error(`frame-profile: ${error.message}`);
    process.exitCode = 1;
  }
}
