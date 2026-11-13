// Native Claude approval fixture. No inference, credentials, or third-party SDK.
const fs = require("node:fs");
const crypto = require("node:crypto");
const readline = require("node:readline");
const path = require("node:path");
if (process.argv.includes("--version")) {
  console.log("2.1.322 (Claude Code)");
  process.exit(0);
}
const log = path.join(process.cwd(), "claude-requests.jsonl");
const emit = value => process.stdout.write(JSON.stringify(value) + "\n");
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
    if (text.includes("approval-fixture")) {
      emit({ type: "control_request", request_id: crypto.randomUUID(), request: {
        subtype: "can_use_tool", tool_name: "Bash", tool_use_id: crypto.randomUUID(),
        input: { command: "echo fixture" }, title: "Run fixture command?",
      } });
    } else {
      scheduleFinish();
    }
  } else if (record.type === "control_response") {
    scheduleFinish();
  }
});
