// Local Pi RPC fixture. It never contacts a provider.
const fs = require("node:fs");
const path = require("node:path");
const crypto = require("node:crypto");
const readline = require("node:readline");
const http = require("node:http");
const args = process.argv.slice(2);
const promptIndex = args.indexOf("--append-system-prompt");
if (promptIndex >= 0 && args[promptIndex + 1] !== undefined) {
  fs.appendFileSync(path.join(process.cwd(), "fixture-system-prompt.jsonl"),
    JSON.stringify(args[promptIndex + 1]) + "\n");
}
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
async function call(name, args) {
  const body = JSON.stringify({ jsonrpc: "2.0", id: 9, method: "tools/call", params: { name, arguments: args } });
  return await new Promise(resolve => {
    const req = http.request(process.env.SHINA_MCP_URL, { method: "POST", headers: { Authorization: `Bearer ${process.env.SHINA_MCP_TOKEN}`, "Content-Type": "application/json", "Content-Length": Buffer.byteLength(body) } }, res => {
      let text = ""; res.on("data", data => { text += data; });
      res.on("end", () => resolve(text));
    });
    req.on("error", error => resolve(`error: ${error.message}`)); req.end(body);
  });
}
async function delegate() {
  const text = await call("delegate_task", { title: "Fixture child", prompt: "child-fixture" });
  if (JSON.parse(text).result?.isError) throw new Error("Fixture delegation failed");
}
async function workflow() {
  for (const name of ["orchestrator_capabilities", "list_threads"]) {
    const text = await call(name, {});
    fs.appendFileSync(path.join(process.cwd(), "pi-mcp-dispatch.jsonl"),
      JSON.stringify({ name, url: process.env.SHINA_MCP_URL,
        authorization: `Bearer ${process.env.SHINA_MCP_TOKEN}`, result: text }) + "\n");
  }
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
    fs.appendFileSync(path.join(process.cwd(), "fixture-behavior.jsonl"), JSON.stringify({
      message: request.message, streamingBehavior: request.streamingBehavior ?? null }) + "\n");
    messages.push({ role: "user", content: [{ type: "text", text: request.message }] }); save();
    running = true;
    // The provider has durably accepted and saved this input, but the server
    // dies before it observes the prompt response.
    if (request.message.includes("peer-crash-after-accept-fixture")) return;
    response(request, {}); emit({ type: "agent_start" });
    // Keep a remote provider turn active while the integration test issues its MCP reads.
    if (request.message.includes("peer-documents-hold-fixture")) return;
    if (request.message.includes("peer-steer-race-fixture")) {
      const release = process.env.SHINA_TEST_STEER_END || path.join(process.cwd(), "peer-steer-end");
      const ended = process.env.SHINA_TEST_STEER_ENDED || path.join(process.cwd(), "peer-steer-ended");
      const armed = process.env.SHINA_TEST_STEER_ARMED || path.join(process.cwd(), "peer-steer-armed");
      fs.writeFileSync(armed, "armed\n");
      const timer = setInterval(() => {
        if (!fs.existsSync(release)) return;
        clearInterval(timer);
        const message = { role: "assistant", content: [{ type: "text", text: "Steer race fixture ended" }], stopReason: "stop", timestamp: Date.now() };
        messages.push(message); save();
        emit({ type: "message_end", message });
        running = false; emit({ type: "agent_end", messages }); emit({ type: "agent_settled" });
        fs.writeFileSync(ended, "ended\n");
      }, 25);
      return;
    }
    // An integration test owns settlement through a later steer or abort, not a timer.
    if (request.message.includes("queue-wait-fixture")) return;
    if (request.message.includes("stale-question-fixture")) {
      emit({ type: "extension_ui_request", id: "question-a", method: "input",
        questions: [{ id: "one", question: "Answer question A?", options: [] }] });
      return;
    }
    if (request.message === "dismiss-question-fixture") {
      emit({ type: "extension_ui_request", id: "dismiss-question", method: "input",
        responseCapability: { type: "message" }, responseMode: "message",
        questions: [{ id: "one", question: "Keep this question?", options: [] }] });
      running = false; emit({ type: "agent_end", messages }); emit({ type: "agent_settled" });
      return;
    }
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
    if (request.message.includes("shina-dispatch-fixture")) await workflow();
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
  } else if (request.type === "extension_ui_response") {
    fs.appendFileSync(path.join(process.cwd(), "fixture-question-replies.jsonl"), JSON.stringify(request) + "\n");
    response(request, {});
    if (request.id === "question-a") {
      // Advance to a different request before any delayed answer A arrives.
      emit({ type: "extension_ui_request", id: "question-b", method: "input",
        questions: [{ id: "one", question: "Answer question B?", options: [] }] });
    } else if (request.id === "question-b") {
      const text = `Question B accepted ${JSON.stringify(request.value ?? null)}`;
      const message = { role: "assistant", content: [{ type: "text", text }], stopReason: "stop", timestamp: Date.now() };
      messages.push(message); save();
      emit({ type: "message_end", message });
      running = false; emit({ type: "agent_end", messages }); emit({ type: "agent_settled" });
    }
  } else if (request.type === "abort") { running = false; response(request, {}); emit({ type: "agent_settled", aborted: true }); }
  else if (request.type === "clear_queue") response(request, { steering: [], followUp: [] });
  else response(request, {});
});
