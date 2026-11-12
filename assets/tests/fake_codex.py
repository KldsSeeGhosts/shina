"""Owned app-server fixture. No provider config, credentials or network access."""
import argparse
import json
import sys
import threading

parser = argparse.ArgumentParser()
parser.add_argument("--log", required=True)
parser.add_argument("--auth-denial", action="store_true")
parser.add_argument("--compact-failure", action="store_true")
parser.add_argument("--compact-rejection", action="store_true")
parser.add_argument("--hold-compaction", action="store_true")
parser.add_argument("--automatic-compaction", action="store_true")
parser.add_argument("--legacy-compaction", action="store_true")
args = parser.parse_args()
initialized = False
thread_id = "codex-fixture-thread"
turn_id = "codex-turn"
pending = None
turn_serial = 0
compacting = False
output_lock = threading.Lock()


def send(value):
    with output_lock:
        print(json.dumps(value), flush=True)


def notify(method, params):
    send({"method": method, "params": {"threadId": thread_id, **params}})


def complete(status="completed"):
    turn = {"id": turn_id, "status": status, "items": []}
    if status == "failed":
        turn["error"] = {"message": "Fixture compaction failure"}
    notify("turn/completed", {"turn": turn})


def finish_compaction(automatic=False):
    global compacting
    if not compacting:
        return
    compacting = False
    if args.compact_failure:
        # Failed compact turns can omit item/completed.
        notify("error", {"willRetry": False, "error": {"message": "Fixture compaction failure"}})
        complete("failed")
        return
    if not args.legacy_compaction:
        notify("item/completed", {"item": {"type": "contextCompaction", "id": turn_id + "-compact"}})
    notify("thread/compacted", {"turnId": turn_id})
    if automatic:
        stream()
    else:
        complete()


def start_compaction(automatic=False):
    global compacting
    compacting = True
    if not args.legacy_compaction:
        notify("item/started", {"item": {"type": "contextCompaction", "id": turn_id + "-compact"}})
    if not args.hold_compaction:
        # Allow a racing prompt while the native turn is active.
        timer = threading.Timer(0.3, finish_compaction, args=(automatic,))
        timer.daemon = True
        timer.start()


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
    elif method == "skills/list":
        assert len(params["cwds"]) == 1
        send({"id": request_id, "result": {"data": [{"cwd": params["cwds"][0], "skills": [
            {"name": "fixture-skill", "description": "Fixture skill", "path": "/fixture/SKILL.md", "enabled": True},
            {"name": "disabled-skill", "path": "/fixture/disabled.md", "enabled": False}
        ]}]}})
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
                {"type": "agentMessage", "id": "old-answer", "text": "Prior reply"},
                {"type": "contextCompaction", "id": "old-compaction"}
            ]}]
        send({"id": request_id, "result": {"thread": {"id": thread_id, "turns": turns},
             "model": "fixture-model", "reasoningEffort": "high"}})
    elif method == "thread/compact/start":
        assert params == {"threadId": thread_id}
        if args.compact_rejection:
            send({"id": request_id, "error": {"code": -32000, "message": "Fixture compaction rejection"}})
            continue
        turn_serial += 1
        turn_id = "codex-turn-" + str(turn_serial)
        send({"id": request_id, "result": {}})
        notify("turn/started", {"turn": {"id": turn_id, "status": "inProgress"}})
        start_compaction()
    elif method == "turn/start":
        assert params["threadId"] == thread_id
        assert not compacting, "Prompt raced compaction"
        turn_serial += 1
        turn_id = "codex-turn-" + str(turn_serial)
        send({"id": request_id, "result": {"turn": {"id": turn_id, "status": "inProgress"}}})
        notify("turn/started", {"turn": {"id": turn_id, "status": "inProgress"}})
        text = "\n".join(item.get("text", "") for item in params["input"])
        notify("item/completed", {"item": {"type": "userMessage", "id": "user",
               "content": params["input"]}})
        if args.automatic_compaction:
            start_compaction(automatic=True)
            continue
        elif "interrupt" in text:
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
        compacting = False
        send({"id": request_id, "result": {}})
        complete("interrupted")
    elif method == "turn/steer":
        assert not compacting, "Steering raced compaction"
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
