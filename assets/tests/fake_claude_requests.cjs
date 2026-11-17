// Native Claude approval fixture. No inference, credentials, or third-party SDK.
const fs = require("node:fs");
const crypto = require("node:crypto");
const http = require("node:http");
const readline = require("node:readline");
const path = require("node:path");
if (process.argv.includes("--version")) {
  console.log("2.1.322 (Claude Code)");
  process.exit(0);
}
const log = path.join(process.cwd(), "claude-requests.jsonl");
const emit = value => process.stdout.write(JSON.stringify(value) + "\n");
// The inline --mcp-config entry this session launched with; headers are reported
// env-expanded the way the real CLI reports mcp_status.
const mcpArg = process.argv.indexOf("--mcp-config");
const shinaEntry = mcpArg < 0 ? {} : (JSON.parse(process.argv[mcpArg + 1]).mcpServers || {}).shina || {};
const expand = value => value.replace(/\$\{(\w+)\}/g, (match, name) => process.env[name] || "");
const shinaUrl = shinaEntry.url;
const shinaToken = process.env.SHINA_MCP_TOKEN || "";
const shinaHeaders = Object.fromEntries(
  Object.entries(shinaEntry.headers || {}).map(([name, value]) => [name, expand(value)]));
// The thread's token-scoped endpoint, the same record fake_pi writes.
if (shinaUrl && shinaToken) {
  fs.writeFileSync(path.join(process.cwd(), "fixture-mcp.json"),
    JSON.stringify({ url: shinaUrl, token: shinaToken }));
}
const callbacks = {};
const dispatch = (name, input, done) => {
  const body = JSON.stringify({ jsonrpc: "2.0", id: 1, method: "tools/call",
    params: { name: name.slice("mcp__shina__".length), arguments: input } });
  const request = http.request(shinaUrl, { method: "POST",
    headers: { authorization: "Bearer " + shinaToken,
      "content-type": "application/json", accept: "application/json" } },
    response => {
      let data = "";
      response.on("data", chunk => data += chunk);
      response.on("end", () => done(data));
    });
  request.on("error", error => done("dispatch failed: " + error.message));
  request.end(body);
};
const ask = (name, input, hook) => {
  const id = "callback-" + name;
  callbacks[id] = { name, input };
  emit({ type: "control_request", request_id: id, request: hook ? {
    subtype: "hook_callback", callback_id: "shina-policy",
    input: { hook_event_name: "PreToolUse", tool_name: name, tool_input: input,
      tool_use_id: id } }
    : { subtype: "can_use_tool", tool_name: name, input, tool_use_id: id } });
};
let completionTimer;
const finish = () => {
  emit({ type: "assistant", uuid: crypto.randomUUID(),
    parent_tool_use_id: null, message: { role: "assistant", model: "default",
      content: [{ type: "text", text: "Fixture complete" }], usage: {} } });
  emit({ type: "result", subtype: "success", is_error: false, num_turns: 1,
    usage: {}, modelUsage: {} });
};
const scheduleFinish = () => {
  clearTimeout(completionTimer);
  completionTimer = setTimeout(finish, 100);
};
readline.createInterface({ input: process.stdin }).on("line", line => {
  const record = JSON.parse(line);
  fs.appendFileSync(log, JSON.stringify(record) + "\n");
  if (record.type === "control_request") {
    const request = record.request;
    const data = request.subtype === "initialize" ? {
      hook_on_failure_supported: true, hooks_applied: true, commands: [],
      models: [{ value: "default", displayName: "Claude fixture" }],
    } : request.subtype === "mcp_status" ? {
      mcpServers: [{ name: "shina", status: "connected", scope: "dynamic",
        config: { type: "http", url: shinaUrl, headers: shinaHeaders } }],
    } : {};
    emit({ type: "control_response", response: { subtype: "success",
      request_id: record.request_id, response: data } });
    if (request.subtype === "interrupt") {
      clearTimeout(completionTimer);
      finish();
    }
  } else if (record.type === "user") {
    emit({ type: "system", subtype: "init", model: "default",
      capabilities: ["interrupt_receipt_v1", "interrupt_cancel_queued_v1"] });
    emit(record);
    const text = record.message.content.filter(x => x.type === "text").map(x => x.text).join("\n");
    if (text.includes("question-file-fixture")) {
      emit({ type: "control_request", request_id: crypto.randomUUID(), request: {
        subtype: "can_use_tool", tool_name: "AskUserQuestion", tool_use_id: crypto.randomUUID(),
        input: { questions: [{ header: "File", question: "Attach a file.", options: [] }] },
      } });
    } else if (text.includes("approval-fixture")) {
      emit({ type: "control_request", request_id: crypto.randomUUID(), request: {
        subtype: "can_use_tool", tool_name: "Bash", tool_use_id: crypto.randomUUID(),
        input: { command: "echo fixture" }, title: "Run fixture command?",
      } });
    } else if (text.includes("shina-dispatch-fixture")) {
      ask("mcp__shina__orchestrator_capabilities", {}, false);
      ask("mcp__shina__list_threads", {}, true);
      ask("mcp__shina__exec_shell", { command: "echo hi" }, false);
      ask("mcp__shina_evil__list_threads", {}, true);
    } else {
      scheduleFinish();
    }
  } else if (record.type === "control_response") {
    const callback = callbacks[record.response.request_id];
    delete callbacks[record.response.request_id];
    const allowed = record.response.response.behavior === "allow" ||
      (record.response.response.hookSpecificOutput || {}).permissionDecision === "allow";
    if (callback && allowed && callback.name.startsWith("mcp__shina__")) {
      dispatch(callback.name, callback.input, result => {
        fs.appendFileSync(log, JSON.stringify({ dispatch: callback.name, result }) + "\n");
        emit({ type: "user", uuid: crypto.randomUUID(), parent_tool_use_id: null,
          message: { role: "user", content: [{ type: "tool_result",
            tool_use_id: record.response.request_id, content: result }] } });
        scheduleFinish();
      });
    } else {
      if (callback) {
        fs.appendFileSync(log, JSON.stringify({ dispatch: callback.name,
          result: "", denied: true }) + "\n");
      }
      scheduleFinish();
    }
  }
});
