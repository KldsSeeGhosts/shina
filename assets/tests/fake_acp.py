"""Local JSONL ACP fixture. No providers, network, login, or non-fixture files."""
import argparse
import json
import sys
import time

args = argparse.ArgumentParser()
args.add_argument("--log", required=True)
args.add_argument("--stdio", action="store_true")
args.add_argument("--no-load", action="store_true")
args.add_argument("--resume", action="store_true")
args.add_argument("--auth-denied", action="store_true")
args.add_argument("--delay-seconds", type=float, default=0)
args.add_argument("--no-history", action="store_true")
opts = args.parse_args()
session = "fixture-session"
pending = None
permission_prompt = None


def send(value):
    print(json.dumps({"jsonrpc": "2.0", **value}), flush=True)


def result(req_id, value):
    send({"id": req_id, "result": value})


def update(session_update, **value):
    send({"method": "session/update", "params": {
        "sessionId": session, "update": {"sessionUpdate": session_update, **value}}})


def finish(req_id, text="Hello ACP"):
    time.sleep(opts.delay_seconds)
    update("agent_message_chunk", content={"type": "text", "text": text[:5]})
    update("agent_message_chunk", content={"type": "text", "text": text[5:]})
    result(req_id, {"stopReason": "end_turn"})


for line in sys.stdin:
    record = json.loads(line)
    with open(opts.log, "a", encoding="utf8") as log:
        log.write(json.dumps(record) + "\n")
    method = record.get("method")
    req_id = record.get("id")
    params = record.get("params", {})
    if method == "initialize":
        assert params["protocolVersion"] == 1
        assert params["clientCapabilities"]["fs"]["readTextFile"] is False
        assert params["clientCapabilities"]["terminal"] is False
        result(req_id, {"protocolVersion": 1, "agentCapabilities": {
            "loadSession": not opts.no_load and not opts.resume,
            "sessionCapabilities": {"resume": {}} if opts.resume else {},
            "promptCapabilities": {"image": True},
            "mcpCapabilities": {"http": not opts.stdio, "stdio": True}}})
    elif method in ("session/new", "session/load", "session/resume"):
        if opts.auth_denied:
            send({"id": req_id, "error": {"code": -32000, "message": "Authentication required"}})
            continue
        assert params["cwd"]
        mcp = params["mcpServers"][0]
        assert mcp["name"] == "shina"
        if opts.stdio:
            assert mcp["args"] == ["--mcp-bridge"]
            assert {v["name"] for v in mcp["env"]} == {"SHINA_MCP_URL", "SHINA_MCP_TOKEN"}
        else:
            assert mcp["type"] == "http"
            assert mcp["url"].startswith("http://127.0.0.1:")
            assert mcp["headers"][0]["value"].startswith("Bearer ")
        if method != "session/new":
            assert params["sessionId"] == session
            if not opts.no_history:
                update("user_message_chunk", content={"type": "text", "text": "Earlier user"})
                update("agent_message_chunk", content={"type": "text", "text": "Earlier reply"})
        update("available_commands_update", availableCommands=[{
            "name": "fixture", "description": "A fake command"}])
        update("plan", entries=[{"content": "Read fixture", "priority": "high", "status": "pending"}])
        result(req_id, {"sessionId": session, "models": {
            "currentModelId": "fixture-model",
            "availableModels": [{"modelId": "fixture-model", "name": "Fixture model"}]},
            "modes": {"currentModeId": "code", "availableModes": [{"id": "code", "name": "Code"}]}})
    elif method == "session/prompt":
        assert params["sessionId"] == session
        blocks = params["prompt"]
        assert all(b["type"] in ("text", "image") for b in blocks)
        text = "\n".join(b["text"] for b in blocks if b["type"] == "text" and
                         not b["text"].startswith("<shina_context>"))
        if text == "hold":
            pending = req_id
            update("agent_message_chunk", content={"type": "text", "text": "Waiting"})
        elif text == "choice-only":
            permission_prompt = req_id
            send({"id": 71, "method": "session/request_permission", "params": {
                "sessionId": session, "toolCall": {"toolCallId": "interaction_fixture",
                    "title": "Choose **one** option.", "kind": "read"},
                "options": [{"kind": "allow_once", "optionId": "choice-1", "name": "First"},
                            {"kind": "reject_once", "optionId": "choice-2", "name": "Second"}]}})
        elif text == "permission":
            permission_prompt = req_id
            update("agent_thought_chunk", content={"type": "text", "text": "Checking"})
            update("tool_call", toolCallId="fixture-tool", title="Write fixture", kind="edit",
                   status="pending", rawInput={"path": "fixture.txt"})
            send({"id": 71, "method": "session/request_permission", "params": {
                "sessionId": session, "toolCall": {"toolCallId": "fixture-tool",
                    "title": "Write fixture", "kind": "edit", "rawInput": {"path": "fixture.txt"}},
                "options": [{"kind": "allow_once", "optionId": "yes", "name": "Allow"},
                            {"kind": "reject_once", "optionId": "no", "name": "Decline"}]}})
        else:
            finish(req_id)
    elif method == "session/cancel":
        if pending:
            result(pending, {"stopReason": "cancelled"})
            pending = None
    elif method == "session/set_model":
        assert params["modelId"] == "fixture-model"
        result(req_id, {})
    elif req_id == 71 and "result" in record:
        assert record["result"]["outcome"]["outcome"] in ("selected", "cancelled")
        update("tool_call_update", toolCallId="fixture-tool", status="completed",
               content=[{"type": "content", "content": {"type": "text", "text": "Written"}}])
        finish(permission_prompt)
    elif req_id is not None:
        send({"id": req_id, "error": {"code": -32601, "message": "Unknown fixture method"}})
