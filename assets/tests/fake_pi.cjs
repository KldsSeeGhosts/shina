// Local Pi RPC fixture. It never contacts a provider.
const fs = require("node:fs");
const path = require("node:path");
const crypto = require("node:crypto");
const readline = require("node:readline");
const http = require("node:http");
const args = process.argv.slice(2);
const sessionIndex = args.indexOf("--session");
const ephemeral = args.includes("--no-session");
const session = sessionIndex >= 0 ? args[sessionIndex + 1] : path.join(process.cwd(), `fixture-${crypto.randomUUID()}.json`);
let messages = sessionIndex >= 0 && fs.existsSync(session) ? JSON.parse(fs.readFileSync(session, "utf8")) : [];
let running = false;
let thinking = "off";
const model = { provider: "cpa", id: "fixture", name: "Fixture", input: ["text"], reasoning: false, contextWindow: 200000 };
const emit = (record) => process.stdout.write(JSON.stringify(record) + "\n");
const response = (request, data) => emit({ type: "response", id: request.id, command: request.type, success: true, data });
const save = () => { if (!ephemeral) fs.writeFileSync(session, JSON.stringify(messages)); };
if (process.env.SHINA_MCP_URL) fs.writeFileSync(path.join(process.cwd(), "fixture-mcp.json"), JSON.stringify({
  url: process.env.SHINA_MCP_URL, token: process.env.SHINA_MCP_TOKEN
}));
async function delegate() {
  const body = JSON.stringify({ jsonrpc: "2.0", id: 1, method: "tools/call", params: { name: "delegate_task", arguments: { title: "Fixture child", prompt: "child-fixture" } } });
  await new Promise((resolve, reject) => {
    const req = http.request(process.env.SHINA_MCP_URL, { method: "POST", headers: { Authorization: `Bearer ${process.env.SHINA_MCP_TOKEN}`, "Content-Type": "application/json", "Content-Length": Buffer.byteLength(body) } }, res => {
      let text = ""; res.on("data", data => { text += data; });
      res.on("end", () => { if (JSON.parse(text).result?.isError) reject(new Error("Fixture delegation failed")); else resolve(); });
    });
    req.on("error", reject); req.end(body);
  });
}
readline.createInterface({ input: process.stdin }).on("line", async line => {
  const request = JSON.parse(line);
  if (request.type === "get_state") response(request, { sessionFile: ephemeral ? null : session, isStreaming: running, model, thinkingLevel: thinking });
  else if (request.type === "get_entries") {
    // Shina normalizes session entries at its adapter boundary.
    emit({ type: "response", id: request.id, command: "get_messages", success: true, data: { messages } });
  } else if (request.type === "get_messages") response(request, { messages });
  else if (request.type === "get_available_models") response(request, { models: [model] });
  else if (request.type === "get_available_thinking_levels") response(request, { levels: ["off"] });
  else if (request.type === "get_commands") response(request, { commands: [] });
  else if (request.type === "get_session_stats") response(request, { contextUsage: { tokens: 10, contextWindow: 200000, percent: 0.005 } });
  else if (request.type === "set_model") response(request, model);
  else if (request.type === "set_thinking_level") { thinking = request.level; response(request, { level: thinking }); }
  else if (request.type === "prompt") {
    fs.appendFileSync(path.join(process.cwd(), "fixture-prompts.jsonl"), JSON.stringify(request.message) + "\n");
    messages.push({ role: "user", content: [{ type: "text", text: request.message }] }); save();
    running = true; response(request, {}); emit({ type: "agent_start" });
    // An integration test owns settlement through a later steer or abort, not a timer.
    if (request.message === "queue-wait-fixture") return;
    if (request.message === "history-stream-fixture") {
      emit({ type: "message_start", message: { role: "assistant" } });
      let text = "";
      let tick = 0;
      const timer = setInterval(() => {
        const delta = `stream-${String(tick++).padStart(3, "0")} 😀\n`;
        text += delta;
        emit({ type: "message_update", assistantMessageEvent: { type: "text_delta", contentIndex: 0, delta } });
        if (tick === 250) {
          clearInterval(timer);
          const message = { role: "assistant", content: [{ type: "text", text }], stopReason: "stop", timestamp: Date.now() };
          messages.push(message); save();
          emit({ type: "message_end", message });
          running = false; emit({ type: "agent_end", messages }); emit({ type: "agent_settled" });
        }
      }, 40);
      return;
    }
    if (request.message === "delegate-fixture") await delegate();
    if (request.message.includes("remote-confirm-fixture")) {
      emit({ type: "extension_ui_request", id: "remote-confirm", method: "confirm", title: "Run fixture command?", message: "A remote confirmation" });
      return;
    }
    setTimeout(() => {
      const text = request.message.includes("remote-long-fixture")
        ? "START FACT\n" + "界".repeat(5500) + "\nMIDDLE FACT\n" + "x".repeat(5500) + "\nEND FACT"
        : "Fixture complete";
      const message = { role: "assistant", content: [{ type: "text", text }], stopReason: "stop", timestamp: Date.now() };
      messages.push(message); save();
      fs.writeFileSync(path.join(process.cwd(), "fixture-finished-at"), String(message.timestamp));
      emit({ type: "message_end", message });
      running = false; emit({ type: "agent_end", messages }); emit({ type: "agent_settled" });
    }, 600);
  } else if (request.type === "abort") { running = false; response(request, {}); emit({ type: "agent_settled", aborted: true }); }
  else if (request.type === "clear_queue") response(request, { steering: [], followUp: [] });
  else response(request, {});
});
