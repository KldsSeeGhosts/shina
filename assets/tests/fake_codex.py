"""Owned app-server fixture. No provider config, credentials or network access."""
import argparse
import json
import sys

parser = argparse.ArgumentParser()
parser.add_argument("--log", required=True)
parser.add_argument("--auth-denial", action="store_true")
args = parser.parse_args()
initialized = False
thread_id = "codex-fixture-thread"
turn_id = "codex-turn"
pending = None


def send(value):
    print(json.dumps(value), flush=True)


def notify(method, params):
    send({"method": method, "params": {"threadId": thread_id, **params}})


def complete(status="completed"):
    notify("turn/completed", {"turn": {"id": turn_id, "status": status, "items": []}})


def stream():
    reasoning = {"type": "reasoning", "id": "r", "summary": ["Checking"], "content": []}
    notify("item/started", {"item": reasoning})
    notify("item/reasoning/summaryTextDelta", {"itemId": "r", "summaryIndex": 0, "delta": "Checking"})
    notify("item/completed", {"item": reasoning})
    command = {"type": "commandExecution", "id": "cmd", "command": "echo fixture", "cwd": ".",
               "status": "inProgress", "aggregatedOutput": ""}
    notify("item/started", {"item": command})
    notify("item/commandExecution/outputDelta", {"itemId": "cmd", "delta": "fixture"})
    command.update(status="completed", aggregatedOutput="fixture", exitCode=0)
    notify("item/completed", {"item": command})
    notify("turn/plan/updated", {"explanation": "Fixture", "plan": [{"step": "Reply", "status": "completed"}]})
    notify("item/agentMessage/delta", {"itemId": "answer", "delta": "Hello "})
    notify("item/agentMessage/delta", {"itemId": "answer", "delta": "Codex"})
    notify("item/completed", {"item": {"type": "agentMessage", "id": "answer", "text": "Hello Codex"}})
    notify("thread/tokenUsage/updated", {"turnId": turn_id, "tokenUsage": {
        "last": {"totalTokens": 1234}, "total": {"totalTokens": 999999}, "modelContextWindow": 200000}})
    complete()


for line in sys.stdin:
    record = json.loads(line)
    with open(args.log, "a", encoding="utf-8") as log:
        log.write(json.dumps(record) + "\n")
    method = record.get("method")
    params = record.get("params", {})
    request_id = record.get("id")
    if method == "initialize":
        assert not initialized
        assert params["clientInfo"]["name"] == "shina"
        initialized = True
        send({"id": request_id, "result": {"userAgent": "fake-codex/0.167.1"}})
    elif method == "initialized":
        assert initialized
    elif method == "model/list":
        send({"id": request_id, "result": {"data": [{
            "id": "fixture-model", "model": "fixture-model", "displayName": "Fixture Codex",
            "hidden": False, "isDefault": True, "defaultReasoningEffort": "high",
            "supportedReasoningEfforts": [{"reasoningEffort": "low"}, {"reasoningEffort": "high"}],
            "inputModalities": ["text", "image"]
        }], "nextCursor": None}})
    elif method in ("thread/start", "thread/resume"):
        assert initialized
        assert "Shina" in params["developerInstructions"]
        assert params["config"]["mcp_servers.shina"]["http_headers"]["Authorization"].startswith("Bearer ")
        if args.auth_denial:
            send({"id": request_id, "error": {"code": -32000, "message": "Authentication required"}})
            continue
        turns = []
        if method == "thread/resume":
            assert params["threadId"] == thread_id
            turns = [{"id": "old", "status": "completed", "items": [
                {"type": "userMessage", "id": "old-user", "content": [{"type": "text", "text": "Prior prompt"}]},
                {"type": "agentMessage", "id": "old-answer", "text": "Prior reply"}
            ]}]
        send({"id": request_id, "result": {"thread": {"id": thread_id, "turns": turns},
             "model": "fixture-model", "reasoningEffort": "high"}})
    elif method == "turn/start":
        assert params["threadId"] == thread_id
        send({"id": request_id, "result": {"turn": {"id": turn_id, "status": "inProgress"}}})
        notify("turn/started", {"turn": {"id": turn_id, "status": "inProgress"}})
        text = "\n".join(item.get("text", "") for item in params["input"])
        notify("item/completed", {"item": {"type": "userMessage", "id": "user",
               "content": params["input"]}})
        if "interrupt" in text:
            continue
        if "approval" in text or "write" in text:
            pending = "write" if "write" in text else "command"
            method = "item/fileChange/requestApproval" if pending == "write" else "item/commandExecution/requestApproval"
            send({"id": 999, "method": method, "params": {
                "threadId": thread_id, "turnId": turn_id, "itemId": "approval",
                "command": "write fixture.txt", "reason": "Fixture approval"}})
        else:
            stream()
    elif method == "turn/interrupt":
        send({"id": request_id, "result": {}})
        complete("interrupted")
    elif method == "turn/steer":
        assert params["expectedTurnId"] == turn_id
        send({"id": request_id, "result": {"turnId": turn_id}})
    elif method is None and request_id == 999:
        allowed = record["result"].get("decision") == "accept"
        if pending == "write" and allowed:
            with open("fixture.txt", "w", encoding="utf-8") as output:
                output.write("allowed")
        stream()
    else:
        send({"id": request_id, "error": {"code": -32601, "message": "Unknown fixture method"}})
