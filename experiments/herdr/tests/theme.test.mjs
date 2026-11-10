import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";

function luminance(hex) {
  const [r, g, b] = hex.slice(1).match(/../g).map((part) => parseInt(part, 16) / 255)
    .map((value) => value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4);
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

for (const appearance of ["light", "dark"]) {
  test(`${appearance} theme keeps normal and secondary text at 4.5:1 contrast`, async () => {
    const { colors } = JSON.parse(await readFile(new URL(`../themes/shina-${appearance}.json`, import.meta.url)));
    for (const [text, background] of [
      ["foreground", "canvas"], ["foreground", "selected"], ["muted", "chrome"],
      ["muted", "canvas"], ["muted", "background"], ["muted", "selected"],
      ["accent", "canvas"], ["solid_foreground", "solid"],
    ]) {
      const a = luminance(colors[text]);
      const b = luminance(colors[background]);
      const contrast = (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
      assert.ok(contrast >= 4.5, `${text} on ${background}: ${contrast.toFixed(2)}`);
    }
  });
}
