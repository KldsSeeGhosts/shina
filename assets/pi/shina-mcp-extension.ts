// Shina's Pi bridge. Shina writes this file and loads it with --extension, so it runs with
// Pi's own module resolution. It connects to the Shina MCP named by SHINA_MCP_URL and
// SHINA_MCP_TOKEN, registers its tools as mcp__shina__<tool>, and tells the model it is
// running inside Shina. Modeled on T3 Code's Pi MCP bridge.
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { request as httpRequest } from "node:http";
import { resolve } from "node:path";
import { Type } from "typebox";

const PROTOCOL = "2025-06-18";
const PREFIX = "mcp__shina__";

// Pure policy, also exercised by the Rust bridge tests without starting Pi.
export function shinaToolDecision(mode: string, builtinRead: boolean, ownBridge: boolean, ownReadOnly: boolean, builtinEdit: boolean): "allow" | "deny" | "confirm" {
  if (mode === "full-access") return "allow";
  if (builtinRead || (mode === "read-only" ? ownBridge : ownReadOnly)) return "allow";
  if (mode === "read-only") return "deny";
  if (mode === "auto-accept-edits" && builtinEdit) return "allow";
  return "confirm";
}

type McpTool = {
  name: string;
  description?: string;
  inputSchema?: Record<string, unknown>;
  outputSchema?: Record<string, unknown>;
  annotations?: { readOnlyHint?: boolean };
};
type ModelContent = { type: "text"; text: string } | { type: "image"; data: string; mimeType: string };

function schema(input: Record<string, unknown> | undefined) {
  const unsafe = (Type as { Unsafe?: (value: unknown) => unknown }).Unsafe;
  if (typeof unsafe === "function" && input !== undefined) {
    return unsafe({ ...input, type: input.type ?? "object", properties: input.properties ?? {} });
  }
  return Type.Object({}, { additionalProperties: true });
}

function modelContent(result: unknown): ModelContent[] {
  const parts = (result as { content?: Array<{ type?: string; text?: string; data?: string; mimeType?: string }> })
    ?.content;
  if (!Array.isArray(parts)) return [{ type: "text", text: JSON.stringify(result ?? null) }];
  const content: ModelContent[] = [];
  for (const part of parts) {
    if (part?.type === "text" && typeof part.text === "string") content.push({ type: "text", text: part.text });
    else if (part?.type === "image" && typeof part.data === "string" && typeof part.mimeType === "string")
      content.push({ type: "image", data: part.data, mimeType: part.mimeType });
  }
  return content;
}

function client(endpoint: string, token: string) {
  let nextId = 1;
  let session: string | undefined;
  const headers = () => ({
    accept: "application/json, text/event-stream",
    authorization: `Bearer ${token}`,
    "content-type": "application/json",
    "mcp-protocol-version": PROTOCOL,
    ...(session ? { "mcp-session-id": session } : {}),
  });
  const request = async (method: string, params?: unknown, signal?: AbortSignal) => {
    // node:http has no fetch/undici five-minute header timeout. A Shina task wait can
    // hold this request for ten minutes; Pi's abort signal still cancels it immediately.
    const response = await new Promise<{ status: number; session?: string; body: string }>((resolve, reject) => {
      const req = httpRequest(endpoint, { method: "POST", headers: headers(), signal }, (res) => {
        const parts: string[] = [];
        res.setEncoding("utf8");
        res.on("data", (part: string) => parts.push(part));
        res.on("error", reject);
        res.on("end", () => resolve({
          status: res.statusCode ?? 500,
          session: res.headers["mcp-session-id"] as string | undefined,
          body: parts.join(""),
        }));
      });
      req.on("error", reject);
      req.end(JSON.stringify({ jsonrpc: "2.0", id: nextId++, method, params }));
    });
    session = response.session ?? session;
    if (response.status < 200 || response.status >= 300)
      throw new Error(`Shina MCP ${method} failed (${response.status}): ${response.body.slice(0, 300)}`);
    const parsed = JSON.parse(response.body);
    if (parsed.error) throw new Error(parsed.error.message ?? `Shina MCP ${method} failed`);
    return parsed.result;
  };
  return {
    async connect(signal?: AbortSignal) {
      const result = await request(
        "initialize",
        { protocolVersion: PROTOCOL, capabilities: {}, clientInfo: { name: "shina-pi", version: "1.0.0" } },
        signal,
      );
      await fetch(endpoint, {
        method: "POST",
        headers: headers(),
        body: JSON.stringify({ jsonrpc: "2.0", method: "notifications/initialized" }),
        signal,
      }).catch(() => undefined);
      return (result?.instructions as string | undefined) ?? "";
    },
    async tools(signal?: AbortSignal): Promise<McpTool[]> {
      return ((await request("tools/list", {}, signal))?.tools ?? []) as McpTool[];
    },
    call(name: string, args: Record<string, unknown>, signal?: AbortSignal) {
      return request("tools/call", { name, arguments: args }, signal);
    },
  };
}

export default async function shinaExtension(pi: ExtensionAPI) {
  // Ported from KSG Code's provider-pi mcpExtensionSource. This is a tool approval policy,
  // not an OS sandbox. The mode is immutable for this process; Shina restarts on changes.
  const mode = process.env.SHINA_RUNTIME_MODE ?? "full-access";
  const readOnlyMcp = new Set<string>();
  pi.on("tool_call", async (event, ctx) => {
    if (mode === "full-access") return;
    const tool = pi.getAllTools().find((tool) => tool.name === event.toolName);
    const builtinRead = (["read", "grep", "find", "ls"].includes(event.toolName) &&
      tool?.sourceInfo?.path === `builtin:${event.toolName}`) ||
      (event.toolName === "tool_search" && tool?.sourceInfo?.path === "builtin:tool-search");
    const bridgePath = process.env.SHINA_PI_BRIDGE_PATH;
    const ownBridge = event.toolName.startsWith(PREFIX) && bridgePath !== undefined &&
      typeof tool?.sourceInfo?.path === "string" && resolve(tool.sourceInfo.path) === resolve(bridgePath);
    const ownReadOnly = ownBridge && readOnlyMcp.has(event.toolName);
    const builtinEdit = ["edit", "write"].includes(event.toolName) &&
      tool?.sourceInfo?.path === `builtin:${event.toolName}`;
    const decision = shinaToolDecision(mode, builtinRead, ownBridge, ownReadOnly, builtinEdit);
    if (decision === "allow") return;
    if (decision === "deny") return {
      block: true,
      reason: `Shina is in Read only mode. ${event.toolName} is blocked. Delegate code changes to a thread with mcp__shina__delegate_task.`,
    };
    const approved = await ctx.ui.confirm(`Allow ${event.toolName}?`,
      JSON.stringify(event.input, null, 2).slice(0, 4_000), { signal: ctx.signal });
    if (!approved) return { block: true, reason: `${event.toolName} was declined in Shina.` };
  });
  pi.on("input", (_event, ctx) => {
    if (ctx.model && ctx.model.provider !== "cpa") {
      ctx.ui.notify("Shina uses CPA proxy models only. Choose a CPA model before starting work.", "error");
      return { action: "handled" };
    }
  });
  const endpoint = process.env.SHINA_MCP_URL;
  const token = process.env.SHINA_MCP_TOKEN;
  if (!endpoint || !token) return;
  const mcp = client(endpoint, token);
  let instructions = "";
  let started: Promise<void> | undefined;

  const start = () => {
    started ??= (async () => {
      const signal = AbortSignal.timeout(10_000);
      instructions = await mcp.connect(signal);
      for (const tool of await mcp.tools(signal)) {
        if (tool.annotations?.readOnlyHint === true) readOnlyMcp.add(PREFIX + tool.name);
        pi.registerTool({
          name: PREFIX + tool.name,
          label: tool.name,
          description: tool.description ?? tool.name,
          parameters: schema(tool.inputSchema),
          ...(tool.outputSchema ? { outputSchema: tool.outputSchema } : {}),
          async execute(_id, params, signal) {
            const result = await mcp.call(tool.name, (params ?? {}) as Record<string, unknown>, signal);
            const isError = (result as { isError?: boolean })?.isError === true;
            return {
              content: modelContent(result),
              details: { server: "shina", tool: tool.name },
              ...((result as { structuredContent?: unknown })?.structuredContent !== undefined
                ? { structuredContent: (result as { structuredContent: unknown }).structuredContent } : {}),
              ...(isError ? { isError: true } : {}),
            };
          },
        });
      }
    })().catch((error) => {
      started = undefined;
      throw error;
    });
    return started;
  };

  // Register at load so the tools exist before the first turn; retry at session start.
  await start().catch(() => undefined);
  pi.on("session_start", async (_event, ctx) => {
    try {
      await start();
    } catch (error) {
      ctx.ui.notify(`Shina tools are unavailable: ${error instanceof Error ? error.message : String(error)}`, "warning");
    }
  });

  // Keep the guidance in the system prompt so leading slash commands still expand.
  pi.on("before_agent_start", (event) => {
    if (!instructions) return undefined;
    const options = event.systemPromptOptions;
    if (options?.sections && typeof options.sections === "object" && options.forceSystemPrompt === undefined) {
      options.sections.shina = instructions;
      return undefined;
    }
    return { systemPrompt: `${event.systemPrompt}\n\n${instructions}` };
  });
}
