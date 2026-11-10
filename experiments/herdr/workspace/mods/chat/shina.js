// Presentation only. Keep the provider's stored transcript unchanged.
export function displayText(text) {
  return String(text ?? "")
    .replace(/\x1b\[[0-?]*[ -/]*[@-~]✻ Worked for [^\r\n]*\x1b\[0m\s*$/, "")
    .replace(/\x1b\][^\x07]*(?:\x07|\x1b\\)/g, "")
    .replace(/\x1b\[[0-?]*[ -/]*[@-~]/g, "");
}

// A send is not accepted merely because the button was pressed. Keep the
// persisted draft until the host acknowledges it, and never erase newer edits.
export async function submitDraft({ text, images }, { send, read, clear }) {
  const result = await send({ text, images });
  const current = read();
  const unchanged = current.text === text && JSON.stringify(current.images) === JSON.stringify(images);
  if (unchanged) clear();
  return { result, cleared: unchanged };
}
