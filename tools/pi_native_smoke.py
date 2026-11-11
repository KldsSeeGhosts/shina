"""Exercise installed Pi's native commands in an isolated profile, without inference.

Run: py tools/pi_native_smoke.py [path/to/pi/dist/cli.js]
A local fixture stream and compaction hook prevent external model or credential requests.
"""
import json
import os
from pathlib import Path
import queue
import subprocess
import sys
import tempfile
import threading
import time


class Rpc:
    def __init__(self, cli, root, extension, source, fork=False):
        env = dict(os.environ, PI_CODING_AGENT_DIR=str(root / "agent"))
        for key in ["SHINA_MCP_URL", "SHINA_MCP_TOKEN", "SHINA_RUNTIME_MODE"]:
            env.pop(key, None)
        self.process = subprocess.Popen(
            ["node", str(cli), "--mode", "rpc", "--offline", "--no-extensions", "--no-mcp",
             "--no-skills", "--no-prompt-templates", "--no-context-files", "--extension", str(extension),
             "--fork" if fork else "--session", str(source)],
            cwd=root, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        )
        self.records = queue.Queue()
        self.events = []
        self.errors = []
        self.serial = 0

        def stdout():
            for line in self.process.stdout:
                try:
                    self.records.put(json.loads(line))
                except ValueError:
                    self.errors.append(line.decode(errors="replace"))
        def stderr():
            for line in self.process.stderr:
                self.errors.append(line.decode(errors="replace"))
        threading.Thread(target=stdout, daemon=True).start()
        threading.Thread(target=stderr, daemon=True).start()

    def call(self, kind, **data):
        self.serial += 1
        request_id = str(self.serial)
        self.process.stdin.write((json.dumps(dict(data, id=request_id, type=kind)) + "\n").encode())
        self.process.stdin.flush()
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            try:
                record = self.records.get(timeout=0.25)
            except queue.Empty:
                if self.process.poll() is not None:
                    raise RuntimeError("Pi exited: " + "".join(self.errors)[-4000:])
                continue
            if record.get("type") == "response" and record.get("id") == request_id:
                if not record.get("success"):
                    raise RuntimeError(f"{kind}: {record.get('error')}")
                return record.get("data")
            self.events.append(record)
        raise TimeoutError(f"{kind}: " + "".join(self.errors)[-4000:])

    def settled(self):
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            try:
                event = self.records.get(timeout=0.25)
            except queue.Empty:
                continue
            self.events.append(event)
            if event.get("type") == "agent_settled":
                return event
        raise TimeoutError("Pi did not settle: " + "".join(self.errors)[-4000:])

    def close(self):
        self.process.stdin.close()
        try:
            self.process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            subprocess.run(["taskkill", "/PID", str(self.process.pid), "/T", "/F"], capture_output=True)
            self.process.wait(timeout=5)


def main():
    default = Path(os.environ["APPDATA"]) / "npm/node_modules/@earendil-works/pi-coding-agent/dist/cli.js"
    cli = Path(sys.argv[1]) if len(sys.argv) > 1 else default
    if not cli.is_file():
        raise SystemExit("Pass the installed Pi dist/cli.js path.")
    with tempfile.TemporaryDirectory(prefix="shina-pi-native-") as directory:
        root = Path(directory)
        agent = root / "agent"
        agent.mkdir()
        (agent / "settings.json").write_text(json.dumps({
            "defaultProvider": "cpa", "defaultModel": "fixture", "defaultThinkingLevel": "off",
            "compaction": {"enabled": False, "keepRecentTokens": 1, "reserveTokens": 1000},
        }))
        bridge = (Path(__file__).resolve().parents[1] / "assets/pi/shina-mcp-extension.ts").as_posix()
        extension = root / "fixture.ts"
        extension.write_text('''import assert from "node:assert/strict";
import {createAssistantMessageEventStream} from "@earendil-works/pi-ai";
import shina from ''' + json.dumps(bridge) + ''';
export default async function(pi) {
  // Call Shina's real gate against a mock UI, never execute an OS tool.
  delete process.env.SHINA_MCP_URL;
  delete process.env.SHINA_MCP_TOKEN;
  for (const mode of ["approval-required", "auto-accept-edits", "full-access"]) {
    process.env.SHINA_RUNTIME_MODE = mode;
    const handlers = new Map();
    const mock = {on:(name, handler) => handlers.set(name,handler), getAllTools:() => [
      {name:"read",sourceInfo:{path:"builtin:read"}}, {name:"write",sourceInfo:{path:"builtin:write"}}
    ]};
    await shina(mock);
    let asked=0;
    const ctx={ui:{confirm:async()=>{asked++;return false;}},signal:undefined};
    const read=await handlers.get("tool_call")({toolName:"read",input:{}},ctx);
    assert.equal(read,undefined);
    const write=await handlers.get("tool_call")({toolName:"write",input:{}},ctx);
    assert.equal(write?.block,mode==="approval-required"?true:undefined);
    assert.equal(asked,mode==="approval-required"?1:0);
    const shell=await handlers.get("tool_call")({toolName:"powershell",input:{command:"never executed"}},ctx);
    assert.equal(shell?.block,mode==="full-access"?undefined:true);
  }
  delete process.env.SHINA_RUNTIME_MODE;
  pi.registerProvider("cpa", {
    baseUrl:"http://127.0.0.1:1",apiKey:"fixture-not-a-credential",api:"openai-completions",
    streamSimple(model,context,options) {
      const stream=createAssistantMessageEventStream();
      const message={role:"assistant",api:"openai-completions",provider:"cpa",model:model.id,
        timestamp:Date.now(),content:[],stopReason:"pending",usage:{input:5,output:3,cacheRead:0,cacheWrite:0,totalTokens:8,
        cost:{input:0,output:0,cacheRead:0,cacheWrite:0,total:0}}};
      queueMicrotask(()=>stream.push({type:"start",partial:message}));
      setTimeout(()=>{
        if(options?.signal?.aborted) {
          message.stopReason="aborted";message.errorMessage="Fixture aborted";
          stream.push({type:"error",reason:"aborted",error:message});
        } else {
          message.content=[{type:"text",text:""}];
          stream.push({type:"text_start",contentIndex:0,partial:message});
          message.content[0].text="Fixture response";
          stream.push({type:"text_delta",contentIndex:0,delta:"Fixture response",partial:message});
          stream.push({type:"text_end",contentIndex:0,content:"Fixture response",partial:message});
          message.stopReason="stop";stream.push({type:"done",reason:"stop",message});
        }
        stream.end();
      },500);
      return stream;
    },
    models:[{id:"fixture",name:"Fixture",reasoning:false,input:["text"],contextWindow:200000,maxTokens:1000,
      cost:{input:0,output:0,cacheRead:0,cacheWrite:0}}]
  });
  pi.registerCommand("fixture-note",{description:"Read-only smoke command",handler:async(args,ctx)=>{
    ctx.ui.notify("fixture-note handled", "info");
  }});
  pi.on("session_before_compact",async(event)=>({compaction:{
    summary:"Native smoke summary. No inference was performed.",
    firstKeptEntryId:event.preparation.firstKeptEntryId,
    tokensBefore:event.preparation.tokensBefore,
  }}));
}
''', encoding="utf-8")
        source = root / "source.jsonl"
        entries = [{"type": "session", "version": 3, "id": "12345678-1234-4234-8234-123456789012",
                    "timestamp": "2026-01-01T00:00:00.000Z", "cwd": str(root)}]
        previous = None
        for i, message in enumerate([
            {"role": "user", "content": "First prompt " * 100},
            {"role": "assistant", "content": [{"type": "text", "text": "First answer " * 100}]},
            {"role": "user", "content": "Second prompt"},
            {"role": "assistant", "content": [{"type": "text", "text": "Second answer"}]},
        ]):
            message["timestamp"] = 1767225600000 + i
            if message["role"] == "assistant":
                message.update(api="openai-completions", provider="cpa", model="fixture", stopReason="stop",
                               usage={"input": 1000, "output": 100, "cacheRead": 0, "cacheWrite": 0,
                                      "totalTokens": 1100, "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0}})
            entry_id = f"entry{i}"
            entries.append({"type": "message", "id": entry_id, "parentId": previous,
                            "timestamp": "2026-01-01T00:00:00.000Z", "message": message})
            previous = entry_id
        source.write_text("\n".join(json.dumps(e) for e in entries) + "\n", encoding="utf-8")
        rpc = Rpc(cli, root, extension, source)
        try:
            state = rpc.call("get_state")
            assert state["model"]["provider"] == "cpa", state
            commands = rpc.call("get_commands")
            assert any(c["name"] == "fixture-note" for c in commands["commands"])
            assert rpc.call("prompt", message="/fixture-note")["disposition"] == "handled"
            compact = rpc.call("compact", customInstructions="Keep decisions")
            assert "Native smoke summary" in compact["summary"], compact
            assert any(e["type"] == "compaction_start" for e in rpc.events)
            assert any(e["type"] == "compaction_end" for e in rpc.events)
            listing = rpc.call("get_entries")
            assert any(e["type"] == "compaction" for e in listing["entries"])
            before = source.read_bytes()
            assert rpc.call("clone")["cancelled"] is False
            cloned = rpc.call("get_state")["sessionFile"]
            assert Path(cloned) != source
            points = rpc.call("get_fork_messages")["messages"]
            assert points
            forked = rpc.call("fork", entryId=points[-1]["entryId"])
            assert not forked["cancelled"]
            assert rpc.call("get_state")["sessionFile"] != cloned
            assert source.read_bytes() == before, "Fork changed the original session"
            assert not any(e["type"] == "agent_start" for e in rpc.events), "Unexpected model work during native operations"
            rpc.events.clear()
            assert rpc.call("prompt", message="Start fixture")["disposition"] == "started"
            assert rpc.call("prompt", message="Steer fixture", streamingBehavior="steer")["disposition"] == "queued"
            assert not rpc.settled()["aborted"]
            users = [e["message"]["content"] for e in rpc.events
                     if e["type"] == "message_start" and e["message"]["role"] == "user"]
            assert len(users) == 2, users
            assert any(e["type"] == "queue_update" for e in rpc.events)
            assert rpc.call("get_session_stats")["totalMessages"] > 0
            assert rpc.call("prompt", message="Cancel fixture")["disposition"] == "started"
            rpc.call("clear_queue")
            rpc.call("abort")
            assert any(e["type"] == "agent_settled" and e.get("aborted") for e in rpc.events)
        finally:
            rpc.close()
        rpc = Rpc(cli, root, extension, source, fork=True)
        try:
            assert Path(rpc.call("get_state")["sessionFile"]) != source
        finally:
            rpc.close()
        print("PASS: discovery, handled commands, native compaction, clone, fork, CLI fork, steering, queue events, abort, permission gates; no external inference")


if __name__ == "__main__":
    main()
