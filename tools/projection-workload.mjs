// One deterministic transcript file for the historical/new Rust and KSG runs.
import { createHash } from "node:crypto";
import { writeFile } from "node:fs/promises";
import { pathToFileURL } from "node:url";

export const FORMAT = "shina-projection-workload-v1";
export const SIZES = [[2, 200, 60], [4, 1000, 300], [4, 4000, 1200]];

export function makeWorkload(sizes = SIZES, updates = 40) {
  if (!Number.isSafeInteger(updates) || updates < 12) throw new Error("At least twelve updates required");
  return {
    format: FORMAT,
    warmup: 10,
    cases: sizes.map(([count, messages, tools]) => {
      if (![count, messages, tools].every(Number.isSafeInteger)
        || count < 2 || messages < 2 || tools < 1 || tools > Math.floor(messages / 2)) {
        throw new Error("Invalid thread/message/tool counts");
      }
      const threads = Array.from({ length: count }, (_, thread) => {
        const id = thread + 1;
        return {
          id,
          messages: Array.from({ length: messages }, (_, index) => ({
            id: `m-${id}-${index}`,
            role: index % 2 === 0 ? "You" : "Pi",
            text: `message ${id}/${index} ${"x".repeat(200)}`,
            images: [],
          })),
          tools: Array.from({ length: tools }, (_, index) => ({
            id: `t-${id}-${index}`,
            name: "read", detail: `file-${index}`, done: true, failed: false,
            output: `output ${id}/${index} ${"y".repeat(200)}`,
            images: [], structured: null, parent_call: null, exit_code: 0, subagents: [],
            anchor: { message: index * 2 + 1, offset: 0 }, block: 0, order: index,
          })),
        };
      });
      const background = threads.at(-1);
      return {
        thread_count: count, messages_per_thread: messages, tools_per_thread: tools,
        total_items: count * (messages + tools),
        threads,
        changes: Array.from({ length: updates }, (_, index) => {
          const field = index % 2 === 0 ? "messages" : "tools";
          const at = background[field].length - 1;
          const item = { ...background[field][at] };
          item[field === "messages" ? "text" : "output"] = `update ${index} ${"z".repeat(200)}`;
          return { thread_id: background.id, field, index: at, item };
        }),
      };
    }),
  };
}

export function fingerprint(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  if (process.argv.length !== 3) throw new Error("Usage: node tools/projection-workload.mjs OUTPUT.json");
  const bytes = Buffer.from(`${JSON.stringify(makeWorkload())}\n`);
  await writeFile(process.argv[2], bytes, { flag: "wx" });
  console.log(JSON.stringify({ path: process.argv[2], sha256: fingerprint(bytes), bytes: bytes.length }));
}
