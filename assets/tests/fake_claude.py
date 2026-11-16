"""Claude Agent SDK 0.3.321 stream-json fixture. No inference or third-party code."""
import json
import os
from pathlib import Path
import re
import sys
import urllib.request
import uuid

args = sys.argv[1:]
log_path = args[args.index("--log") + 1]
session = args[args.index("--resume") + 1] if "--resume" in args else (
    args[args.index("--session-id") + 1] if "--session-id" in args else str(uuid.uuid4()))
callbacks = {}
holding = False

# The inline --mcp-config entry Shina launched this session with. The fixture resolves
# ${VAR} headers the way the real CLI reports them in mcp_status, so Shina's binding
# check sees the endpoint it registered or the foreign one a flag asks for.
mcp_config = json.loads(args[args.index("--mcp-config") + 1]) if "--mcp-config" in args else {}
shina_entry = mcp_config.get("mcpServers", {}).get("shina", {})
shina_url = shina_entry.get("url")
shina_token = os.environ.get("SHINA_MCP_TOKEN", "")


def resolved_headers():
    def expand(value):
        return re.sub(r"\$\{(\w+)\}", lambda match: os.environ.get(match.group(1), ""), value)
    return {name: expand(value) for name, value in shina_entry.get("headers", {}).items()}


def mcp_servers():
    if "--unbound-shina" in args:
        return []
    entry = dict(name="shina", status="connected", scope="dynamic",
                 config=dict(type="http", url=shina_url, headers=resolved_headers()))
    if "--foreign-shina" in args:
        entry["config"] = dict(type="http", url="http://127.0.0.1:9/foreign",
                               headers=dict(Authorization="Bearer foreign-token"))
    if "--duplicate-shina" in args:
        return [entry, dict(entry, config=dict(entry["config"]))]
    return [entry]


def dispatch(name, tool_input):
    # A real tools/call against the thread's endpoint so tests cover authenticated
    # dispatch, not only the permission callback.
    if not shina_url or not shina_token:
        return "dispatch skipped: no endpoint"
    tool = name[len("mcp__shina__"):]
    body = json.dumps(dict(jsonrpc="2.0", id=1, method="tools/call",
                           params=dict(name=tool, arguments=tool_input))).encode()
    request = urllib.request.Request(
        shina_url, data=body,
        headers=dict(authorization="Bearer " + shina_token,
                     **{"content-type": "application/json",
                        "accept": "application/json, text/event-stream"}))
    try:
        return urllib.request.urlopen(request, timeout=15).read().decode()
    except Exception as error:
        return "dispatch failed: " + str(error)
project = Path(os.environ["CLAUDE_CONFIG_DIR"]) / "projects" / "fixture"
project.mkdir(parents=True, exist_ok=True)
history_path = project / (session + ".jsonl")
parent_uuid = None
if history_path.exists():
    for prior in history_path.read_text().splitlines():
        parent_uuid = json.loads(prior)["uuid"]


def emit(record):
    global parent_uuid
    record.setdefault("session_id", session)
    if record.get("type") in ("user", "assistant") and record.get("parent_tool_use_id") is not None:
        subdir = project / session / "subagents"
        subdir.mkdir(parents=True, exist_ok=True)
        with (subdir / "agent-fixture.jsonl").open("a") as history:
            history.write(json.dumps(record) + "\n")
    if record.get("type") in ("user", "assistant") and record.get("parent_tool_use_id") is None:
        if record.get("message", {}).get("content", [{}])[0].get("tool_use_id") == "task":
            record["toolUseResult"] = {"agentId": "fixture"}
        saved = dict(record, parentUuid=parent_uuid, sessionId=session)
        with history_path.open("a") as history:
            history.write(json.dumps(saved) + "\n")
        parent_uuid = record["uuid"]
    print(json.dumps(record), flush=True)


def message(role, content, parent=None, **extra):
    emit(dict(type=role, uuid=str(uuid.uuid4()), parent_tool_use_id=parent,
              message=dict(role=role, id=str(uuid.uuid4()), content=content,
                           usage=dict(input_tokens=100, cache_read_input_tokens=200,
                                      cache_creation_input_tokens=30, output_tokens=10)),
              **extra))


def finish(error=None):
    emit(dict(type="result", subtype="error_during_execution" if error else "success",
              is_error=bool(error), errors=[error] if error else [], queued_turn_count=0,
              usage=dict(input_tokens=9999), modelUsage={"default": dict(inputTokens=999999, contextWindow=200000)}))


def callback(name, tool_input, hook=False):
    # Deterministic ids let tests match each control_response to its tool.
    rid = "callback-" + name
    request = dict(subtype="can_use_tool", tool_name=name, input=tool_input, tool_use_id=rid)
    if hook:
        request = dict(subtype="hook_callback", callback_id="shina-policy",
                       input=dict(hook_event_name="PreToolUse", tool_name=name,
                                  tool_input=tool_input, tool_use_id=rid))
    callbacks[rid] = (name, tool_input)
    emit(dict(type="control_request", request_id=rid, request=request))


def allowed(response):
    return response.get("behavior") == "allow" or \
        response.get("hookSpecificOutput", {}).get("permissionDecision") == "allow"


with open(log_path, "a", encoding="utf8") as log:
    log.write(json.dumps(dict(argv=args, mcp_token=os.environ.get("SHINA_MCP_TOKEN"))) + "\n")
    log.flush()
    for line in sys.stdin:
        record = json.loads(line)
        log.write(json.dumps(record) + "\n")
        log.flush()
        if record["type"] == "control_request":
            request = record["request"]
            subtype = request["subtype"]
            data = {}
            if subtype == "initialize":
                if "--auth-fail" in args:
                    emit(dict(type="control_response", response=dict(subtype="error",
                         request_id=record["request_id"], error="Authentication failed")))
                    continue
                data = dict(hook_on_failure_supported=True, hooks_applied=True,
                            commands=[dict(name="review", description="Review changes")],
                            models=[dict(value="default", displayName="Claude fixture",
                                         supportsEffort=True, supportedEffortLevels=["low", "high"])])
            elif subtype == "interrupt":
                holding = False
                data = dict(still_queued=[], cancelled=[])
            elif subtype == "mcp_status":
                data = dict(mcpServers=mcp_servers())
            emit(dict(type="control_response", response=dict(subtype="success",
                      request_id=record["request_id"], response=data)))
            if subtype == "interrupt":
                finish()
        elif record["type"] == "control_response":
            rid = record["response"]["request_id"]
            name, tool_input = callbacks.pop(rid, ("", {}))
            if name:
                result = ""
                if allowed(record["response"]["response"]) and name.startswith("mcp__shina__"):
                    result = dispatch(name, tool_input)
                    message("user", [dict(type="tool_result", tool_use_id=rid,
                                          content=result)])
                else:
                    message("assistant", [dict(type="text", text="Callback handled")])
                log.write(json.dumps(dict(callback=name, allowed=allowed(
                    record["response"]["response"]), result=result)) + "\n")
                log.flush()
                if not callbacks:
                    finish()
        elif record["type"] == "user":
            content = record["message"]["content"]
            text = next((b["text"] for b in content if b["type"] == "text"), "")
            emit(dict(type="system", subtype="init", model="default",
                      capabilities=["interrupt_receipt_v1", "interrupt_cancel_queued_v1"]))
            emit(record)
            if text.startswith("/compact"):
                emit(dict(type="system", subtype="status", status="compacting"))
                if "fail" in text:
                    finish("Compaction failed")
                elif "hold" in text:
                    holding = True
                else:
                    emit(dict(type="system", subtype="compact_boundary",
                              compact_metadata=dict(trigger="manual", pre_tokens=1000)))
                    finish()
            elif text == "hold":
                holding = True
            elif text == "permission":
                callback("Bash", dict(command="echo fixture"))
            elif text == "coordinate":
                # Real Shina names on both permission paths plus lookalike spoofs.
                callback("mcp__shina__orchestrator_capabilities", {})
                callback("mcp__shina__captain_crew_send", dict(requestId="x"), hook=True)
                callback("mcp__shina__exec_shell", dict(command="echo hi"))
                callback("mcp__shina_evil__list_threads", {}, hook=True)
            elif text == "plan":
                callback("ExitPlanMode", dict(plan="# Plan\nChange the fixture."), hook=True)
            elif text == "question":
                callback("AskUserQuestion", dict(questions=[dict(question="Which file?",
                         header="File", options=[dict(label="a.rs", description="First")],
                         multiSelect=False)]), hook=True)
            elif text == "error":
                finish("Fixture execution failed")
            else:
                mid = str(uuid.uuid4())
                emit(dict(type="stream_event", parent_tool_use_id=None,
                          event=dict(type="message_start", message=dict(id=mid, usage=dict(input_tokens=100)))))
                emit(dict(type="stream_event", parent_tool_use_id=None,
                          event=dict(type="content_block_delta", index=0,
                                     delta=dict(type="thinking_delta", thinking="Checking"))))
                emit(dict(type="stream_event", parent_tool_use_id=None,
                          event=dict(type="content_block_delta", index=1,
                                     delta=dict(type="text_delta", text="Hello Claude"))))
                emit(dict(type="assistant", uuid=str(uuid.uuid4()), parent_tool_use_id=None,
                          user_message_uuid=record["uuid"],
                          message=dict(id=mid, role="assistant",
                          content=[dict(type="thinking", thinking="Checking"),
                                   dict(type="text", text="Hello Claude"),
                                   dict(type="tool_use", id="read", name="Read", input=dict(file_path="fixture.txt"))],
                          usage=dict(input_tokens=100, cache_read_input_tokens=200,
                                     cache_creation_input_tokens=30, output_tokens=10))))
                message("user", [dict(type="tool_result", tool_use_id="read", content="fixture")])
                message("assistant", [dict(type="tool_use", id="task", name="Task",
                                          input=dict(subagent_type="Explore", prompt="Inspect"))])
                message("assistant", [dict(type="tool_use", id="nested", name="Grep",
                                          input=dict(pattern="x"))], parent="task")
                message("user", [dict(type="tool_result", tool_use_id="nested", content="found")], parent="task")
                message("user", [dict(type="tool_result", tool_use_id="task", content="Inspected")])
                message("assistant", [dict(type="tool_use", id="todo", name="TodoWrite",
                                          input=dict(todos=[dict(content="Inspect", status="completed")]))])
                emit(dict(type="rate_limit_event", rate_limit_info=dict(status="allowed_warning", utilization=0.8)))
                finish()
