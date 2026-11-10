export function captainForMachine(name, overrides = {}) {
  const crew = ["ember", "juniper", "sable", "pip"];
  const selected = overrides?.[name ?? "local"];
  if (crew.includes(selected)) return selected;
  if (!name) return "ember";
  let hash = 0;
  for (const letter of name) hash = (hash * 31 + letter.codePointAt(0)) >>> 0;
  return crew[1 + hash % 3];
}
