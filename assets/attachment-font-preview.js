// Ported from KSG Code's files/FontViewer.tsx. This runs only in the isolated document host.
(() => {
  let finished = false;
  document.body.style.visibility = "hidden";
  const fail = (message) => {
    if (finished) return;
    finished = true;
    document.body.style.visibility = "";
    document.body.style.fontFamily = "var(--font-sans)";
    const title = document.createElement("p");
    title.textContent = "No preview for this file";
    const detail = document.createElement("p");
    detail.textContent = `${message} Save it to open in another app.`;
    document.body.replaceChildren(title, detail);
  };
  const timeout = setTimeout(() => fail("The font took too long to load."), 30_000);
  try {
    const bytes = Uint8Array.from(atob("__FONT_DATA__"), (c) => c.charCodeAt(0));
    new FontFace("preview", bytes.buffer).load().then((face) => {
      if (finished) return;
      finished = true;
      clearTimeout(timeout);
      document.fonts.add(face);
      document.body.style.visibility = "";
    }).catch(() => {
      clearTimeout(timeout);
      fail("The font is damaged or unsupported.");
    });
  } catch {
    clearTimeout(timeout);
    fail("The font could not be decoded.");
  }
})();
