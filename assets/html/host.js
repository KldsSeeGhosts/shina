// This trusted host contains no untrusted markup outside the sandboxed srcdoc.
window.shinaState = { height: 80, ready: false, consoleMessages: [] };
window.shinaTheme = theme => {
  window.shinaInitialTheme = theme;
  document.querySelector("iframe").contentWindow.postMessage({
    jsonrpc: "2.0",
    method: "ui/notifications/host-context-changed",
    params: { theme: theme.appearance, styles: { variables: theme.variables } },
  }, "*");
};
document.querySelector("iframe").addEventListener("load", () => window.shinaTheme(window.shinaInitialTheme));
window.addEventListener("message", event => {
  if (event.source !== document.querySelector("iframe").contentWindow) return;
  const envelope = event.data;
  if (!envelope || envelope.jsonrpc !== "2.0") return;
  const data = envelope.params;
  if (envelope.method === "ui/notifications/size-changed" &&
      data && Number.isFinite(data.height) && data.height > 0) {
    const consoleMessages = Array.isArray(data.consoleMessages) ? data.consoleMessages : [];
    window.shinaState = {
      height: Math.min(20000, Math.ceil(data.height)),
      ready: data.ready === true,
      consoleMessages: consoleMessages.slice(0, 20).filter(message => message && typeof message === "object").map(message => ({
        level: ["log", "info", "warning", "error"].includes(message.level) ? message.level : "log",
        text: typeof message.text === "string" ? Array.from(message.text).slice(0, 500).map(character => {
          const code = character.codePointAt(0);
          return code >= 0xd800 && code <= 0xdfff ? "\ufffd" : character;
        }).join("") : "",
      })),
    };
  }
  if (envelope.method === "ui/open-link" && data && typeof data.url === "string" && /^https?:\/\//i.test(data.url) &&
      navigator.userActivation && navigator.userActivation.isActive) {
    window.shinaOpenLink(data.url);
    if (typeof envelope.id === "string" || typeof envelope.id === "number") {
      event.source.postMessage({ jsonrpc: "2.0", id: envelope.id, result: {} }, "*");
    }
  }
});
