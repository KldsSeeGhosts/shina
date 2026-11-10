import { appendFileSync, readFileSync } from "node:fs";
import { createInterface } from "node:readline";

const scenario = JSON.parse(readFileSync("scenario.json", "utf8")).scenario;
if (scenario === "exit") process.exit(2);
const input = createInterface({ input: process.stdin });
input.on("line", (line) => {
  const command = JSON.parse(line);
  appendFileSync("requests.jsonl", JSON.stringify({ ...command, argv: process.argv.slice(2) }) + "\n");
  if (scenario === "hang") return;
  if (scenario === "auth") {
    console.log(JSON.stringify({ type: "response", command: command.type, success: false, error: "401 unauthorized secret-that-must-not-leak" }));
    return;
  }
  const provider = scenario === "direct" ? "other-provider" : "cpa";
  const model = { provider, id: "test-model", name: "Test 模型" };
  const data = command.type === "get_state"
    ? { model, thinkingLevel: "high" }
    : { models: [model, { provider: "direct", id: "must-not-count" }] };
  console.log(JSON.stringify({ type: "response", command: command.type, success: true, data }));
});
