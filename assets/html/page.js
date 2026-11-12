// Runs only in the opaque, sandboxed page. There is no application bridge here.
(() => {
  const messages = [];
  const clip = value => Array.from(String(value)).slice(0, 500).map(character => {
    const code = character.codePointAt(0);
    return code >= 0xd800 && code <= 0xdfff ? "\ufffd" : character;
  }).join("");
  const record = (level, text) => {
    if (messages.length < 20) {
      messages.push({ level, text: clip(text) });
      queueMicrotask(report);
    }
  };
  for (const level of ["log", "debug", "info", "warn", "error", "dir", "dirxml", "table", "trace"]) {
    const original = console[level];
    console[level] = (...args) => {
      original.apply(console, args);
      const text = args.map(value => {
        try { return typeof value === "string" ? value : JSON.stringify(value); }
        catch { return String(value); }
      }).join(" ");
      record(level === "warn" ? "warning" : ["error", "info"].includes(level) ? level : "log", text);
    };
  }
  window.addEventListener("error", event => record("error",
    event.message || "Resource failed to load: " + (event.target.src || event.target.href || "unknown")));
  window.addEventListener("unhandledrejection", event => record("error", event.reason));

  const report = () => parent.postMessage({
    jsonrpc: "2.0",
    method: "ui/notifications/size-changed",
    params: { height: Math.ceil(Math.max(
      document.documentElement.getBoundingClientRect().height,
      document.body ? document.body.getBoundingClientRect().bottom : 0,
    )),
    ready: document.readyState === "complete" &&
      (!document.fonts || document.fonts.status === "loaded"),
    consoleMessages: messages },
  }, "*");
  new ResizeObserver(report).observe(document.documentElement);
  window.addEventListener("load", report);
  if (document.fonts) document.fonts.ready.then(() =>
    requestAnimationFrame(() => requestAnimationFrame(report)));

  window.addEventListener("message", event => {
    if (event.source !== parent || !event.data) return;
    const data = event.data;
    if (data.jsonrpc !== "2.0" || data.method !== "ui/notifications/host-context-changed") return;
    const theme = { appearance: data.params.theme, variables: data.params.styles.variables };
    document.documentElement.style.colorScheme = theme.appearance;
    for (const key in theme.variables) {
      document.documentElement.style.setProperty(key, theme.variables[key]);
    }
  });

  document.addEventListener("click", event => {
    const link = event.composedPath().find(node => node && node.matches && node.matches("a[href]"));
    if (!link) return;
    const href = link.getAttribute("href");
    if (href.startsWith("#")) return;
    event.preventDefault();
    if (!event.isTrusted) return;
    try {
      const url = new URL(link.href.replace(/^shina-(https?):/, "$1:"));
      if (/^https?:$/.test(url.protocol)) parent.postMessage({
        jsonrpc: "2.0", id: "shina-link", method: "ui/open-link", params: { url: url.href },
      }, "*");
    } catch {}
  }, true);
})();
