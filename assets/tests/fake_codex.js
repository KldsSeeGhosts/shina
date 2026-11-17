// Codex app-server fixture. No provider calls, credentials or network access.
// Shina launches it as `node fake_codex.cjs app-server`; it speaks the JSONL
// app-server protocol on stdio and records the thread's Shina MCP endpoint the
// way fake_pi does, so tests drive real tools/call through it.
const fs = require("node:fs");
const path = require("node:path");
const readline = require("node:readline");
if (process.argv.includes("--version")) {
  console.log("codex-cli 0.167.2");
  process.exit(0);
}
const threadId = "codex-fixture-thread";
let turns = 0;
const emit = record => process.stdout.write(JSON.stringify(record) + "\n");
const log = record =>
  fs.appendFileSync(path.join(process.cwd(), "codex-requests.jsonl"),
    JSON.stringify(record) + "\n");
const notify = (method, params) => emit({ method, params: { threadId, ...params } });
readline.createInterface({ input: process.stdin }).on("line", line => {
  const record = JSON.parse(line);
  log(record);
  const method = record.method;
  const params = record.params || {};
  const id = record.id;
  const result = value => emit({ id, result: value });
  if (method === "initialize") return result({ userAgent: "fake-codex/0.167.2" });
  if (method === "initialized") return;
  if (method === "skills/list")
    return result({ data: (params.cwds || []).map(cwd => ({ cwd, skills: [] })) });
  if (method === "account/read") return result({});
  if (method === "account/rateLimits/read")
    return result({ rateLimits: { limitId: "codex",
      primary: { usedPercent: 10, windowDurationMins: 300, resetsAt: 2100000000 } } });
  if (method === "model/list")
    return result({ data: [{ id: "fixture-model", model: "fixture-model",
      displayName: "Fixture Codex", isDefault: true, defaultReasoningEffort: "high",
      supportedReasoningEfforts: [{ reasoningEffort: "low" }, { reasoningEffort: "high" }],
      inputModalities: ["text", "image"] }], nextCursor: null });
  if (method === "thread/turns/list") return result({ data: [], nextCursor: null });
  if (method === "thread/goal/get") return result({ goal: null });
  if (method === "thread/goal/set") return result({ goal: params });
  if (method === "thread/goal/clear") return result({ cleared: true });
  if (method === "thread/settings/update") return result({});
  if (method === "thread/start" || method === "thread/resume" || method === "thread/fork") {
    const server = (params.config || {})["mcp_servers.shina"] || {};
    const authorization = (server.http_headers || {}).Authorization || "";
    if (server.url && authorization.startsWith("Bearer ")) {
      fs.writeFileSync(path.join(params.cwd || process.cwd(), "fixture-mcp.json"),
        JSON.stringify({ url: server.url, token: authorization.slice(7) }));
    }
    return result({ thread: { id: threadId, turns: [], historyMode: "paginated" },
      model: "fixture-model", reasoningEffort: "high" });
  }
  if (method === "turn/start") {
    turns += 1;
    const turnId = `codex-turn-${turns}`;
    result({ turn: { id: turnId, status: "inProgress" } });
    notify("turn/started", { turn: { id: turnId, status: "inProgress" } });
    notify("item/completed", { item: { type: "userMessage", id: "user",
      content: params.input } });
    const text = (params.input || []).map(item => item.text || "").join("\n");
    if (text.includes("hold")) return;
    notify("item/agentMessage/delta", { itemId: "answer", delta: "Hello Codex" });
    notify("item/completed", { item: { type: "agentMessage", id: "answer",
      text: "Hello Codex" } });
    notify("thread/tokenUsage/updated", { turnId, tokenUsage: {
      last: { totalTokens: 10 }, total: { totalTokens: 100 },
      modelContextWindow: 200000 } });
    notify("turn/completed", { turn: { id: turnId, status: "completed", items: [] } });
    return;
  }
  if (method === "turn/steer") return result({ turnId: `codex-turn-${turns}` });
  if (method === "turn/interrupt") {
    result({});
    notify("turn/completed", { turn: { id: `codex-turn-${turns}`,
      status: "interrupted", items: [] } });
    return;
  }
  emit({ id, error: { code: -32601, message: `Unknown fixture method ${method}` } });
});
