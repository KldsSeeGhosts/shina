"""Local JSONL ACP fixture. No providers, network, login, or non-fixture files."""
import argparse
import json
import os
import sys
import time
import urllib.request

args = argparse.ArgumentParser()
args.add_argument("--log", required=True)
args.add_argument("--stdio", action="store_true")
args.add_argument("--no-load", action="store_true")
args.add_argument("--resume", action="store_true")
args.add_argument("--auth-denied", action="store_true")
args.add_argument("--antigravity", action="store_true")
args.add_argument("--discovery", action="store_true")
args.add_argument("--late-catalog", action="store_true")
args.add_argument("--delay-seconds", type=float, default=0)
args.add_argument("--no-history", action="store_true")
args.add_argument("--no-mcp", action="store_true")
args.add_argument("--record-launch", action="store_true")
opts = args.parse_args()

if opts.record_launch and sys.platform == "win32":
    import ctypes

    console_window = ctypes.windll.kernel32.GetConsoleWindow
    console_window.restype = ctypes.c_void_p
    launch = {"pid": os.getpid(), "consoleWindow": console_window()}
    with open(opts.log, "a", encoding="utf8") as log:
        log.write(json.dumps({"fixtureLaunch": launch}) + "\n")

session = "fixture-session"
pending = None
permission_prompt = None
mcp_http = None
session_cwd = None
agy_config = None
agy_mode = None


def agy_options(model="gemini-3.8-flash", level="high"):
    return [{"id": "model", "name": "Model", "category": "model", "type": "select",
             "currentValue": model,
             "options": [{"value": "gemini-3.8-flash", "name": "Gemini 3.8 Flash"},
                         {"value": "gemini-4-carbon", "name": "Gemini 4 Carbon"}]},
            {"id": "thinking_level", "name": "Thinking", "category": "thought_level",
             "type": "select", "currentValue": level,
             "options": [{"value": "low", "name": "Low"},
                         {"value": "medium", "name": "Medium"},
                         {"value": "high", "name": "High"}]}]


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
        assert params["clientCapabilities"]["fs"]["readTextFile"] == (opts.antigravity and not opts.discovery)
        assert params["clientCapabilities"]["fs"]["writeTextFile"] is False
        assert params["clientCapabilities"]["terminal"] is False
        capabilities = {"loadSession": not opts.no_load and not opts.resume,
            "sessionCapabilities": {"resume": {}} if opts.resume else {},
            "promptCapabilities": {"image": True},
            "mcpCapabilities": {"http": not opts.stdio and not opts.no_mcp,
                                "stdio": not opts.no_mcp}}
        if opts.late_catalog:
            capabilities["sessionCapabilities"]["delete"] = {}
        if opts.antigravity:
            assert params["clientInfo"]["name"] == "zed"
            capabilities["_meta"] = {"antigravity": {"systemPrompt": {"append": True}}}
        result(req_id, {"protocolVersion": 1, "agentCapabilities": capabilities})
    elif method == "authenticate":
        assert opts.antigravity and params["methodId"] == "oauth-personal"
        result(req_id, {})
    elif method in ("session/new", "session/load", "session/resume"):
        if opts.auth_denied:
            send({"id": req_id, "error": {"code": -32000, "message": "Authentication required"}})
            continue
        assert params["cwd"]
        session_cwd = params["cwd"]
        metadata_only = params["mcpServers"] == []
        if opts.discovery or metadata_only:
            assert method == "session/new"
            assert metadata_only
        elif opts.stdio:
            mcp = params["mcpServers"][0]
            assert mcp["name"] == "shina"
            assert mcp["args"] == ["--mcp-bridge"]
            assert {v["name"] for v in mcp["env"]} == {"SHINA_MCP_URL", "SHINA_MCP_TOKEN"}
            endpoint = {v["name"]: v["value"] for v in mcp["env"]}
            with open(os.path.join(params["cwd"], "fixture-mcp.json"), "w",
                      encoding="utf8") as out:
                json.dump({"url": endpoint["SHINA_MCP_URL"],
                           "token": endpoint["SHINA_MCP_TOKEN"]}, out)
        else:
            mcp = params["mcpServers"][0]
            assert mcp["name"] == "shina"
            assert mcp["type"] == "http"
            assert mcp["url"].startswith("http://127.0.0.1:")
            assert mcp["headers"][0]["value"].startswith("Bearer ")
            mcp_http = mcp
            with open(os.path.join(params["cwd"], "fixture-mcp.json"), "w",
                      encoding="utf8") as out:
                json.dump({"url": mcp["url"],
                           "token": mcp["headers"][0]["value"][7:]}, out)
        if method != "session/new":
            assert params["sessionId"] == session
            if not opts.no_history:
                update("user_message_chunk", content={"type": "text", "text": "Earlier user"})
                update("agent_message_chunk", content={"type": "text", "text": "Earlier reply"})
        update("available_commands_update", availableCommands=[{
            "name": "fixture", "description": "A fake command"}])
        update("plan", entries=[{"content": "Read fixture", "priority": "high", "status": "pending"}])
        if opts.antigravity:
            if opts.discovery:
                assert "_meta" not in params
            else:
                assert params["_meta"]["antigravity"]["systemPrompt"]["append"]
            agy_config = agy_options()
            result(req_id, {"sessionId": session, "configOptions": agy_config,
                "models": {"currentModelId": "gemini-3.8-flash",
                    "availableModels": [
                        {"modelId": "gemini-3.8-flash-high", "name": "Gemini 3.8 Flash (High)"},
                        {"modelId": "gemini-3.8-flash-medium", "name": "Gemini 3.8 Flash (Medium)"},
                        {"modelId": "gemini-3.8-flash-low", "name": "Gemini 3.8 Flash (Low)"},
                        {"modelId": "gemini-4-carbon", "name": "Gemini 4 Carbon"}]},
                "modes": {"currentModeId": "code", "availableModes": [{"id": "code", "name": "Code"}]}})
        elif opts.late_catalog:
            result(req_id, {"sessionId": session, "models": {
                "currentModelId": "startup",
                "availableModels": [{"modelId": "startup", "name": "Starting up"}]}})
            time.sleep(0.6)
            update("config_option_update", configOptions=[
                {"id": "model", "name": "Model", "type": "select", "category": "model",
                 "currentValue": "deep",
                 "options": [{"group": "Family", "name": "Family", "options": [
                     {"value": "fast", "name": " Fast "},
                     {"value": "deep", "name": "Deep"},
                     {"value": "fast", "name": "Duplicate"}]}]},
                {"id": "runtime_effort", "name": "Thinking", "type": "select",
                 "category": "thinking", "currentValue": "high",
                 "options": [{"value": "high", "name": "High"}]},
            ])
        else:
            result(req_id, {"sessionId": session, "models": {
                "currentModelId": "fixture-model",
                "availableModels": [{"modelId": "fixture-model", "name": "Fixture model"}]},
                "modes": {"currentModeId": "code", "availableModes": [{"id": "code", "name": "Code"}]}})
    elif method == "session/delete":
        assert opts.late_catalog and opts.discovery and params["sessionId"] == session
        result(req_id, {})
    elif method == "session/set_config_option":
        assert opts.antigravity and agy_config
        config_id = params["configId"]
        assert config_id in ("model", "thinking_level")
        for option in agy_config:
            if option["id"] == config_id:
                option["currentValue"] = params["value"]
        result(req_id, {"configOptions": agy_config})
    elif method == "session/set_mode":
        assert opts.antigravity
        agy_mode = params["modeId"]
        result(req_id, {})
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
        elif "shina-dispatch-fixture" in text and mcp_http:
            token = mcp_http["headers"][0]["value"]
            for name in ("orchestrator_capabilities", "list_threads"):
                body = json.dumps({"jsonrpc": "2.0", "id": 9,
                    "method": "tools/call", "params": {"name": name, "arguments": {}}}
                ).encode("utf8")
                request = urllib.request.Request(mcp_http["url"], data=body, method="POST",
                    headers={"Authorization": token, "Content-Type": "application/json"})
                try:
                    with urllib.request.urlopen(request, timeout=10) as response:
                        payload = response.read().decode("utf8")
                except Exception as error:
                    payload = f"error: {error}"
                with open(os.path.join(session_cwd, "acp-mcp-dispatch.jsonl"), "a",
                          encoding="utf8") as out:
                    out.write(json.dumps({"name": name, "url": mcp_http["url"],
                                          "authorization": token, "result": payload}) + "\n")
            update("agent_message_chunk", content={"type": "text", "text": "MCP workflow complete"})
            result(req_id, {"stopReason": "end_turn"})
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
