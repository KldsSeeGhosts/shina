// Compatibility only. Native content rules still block raw HTTP if a page
// removes these setters. The custom scheme's Rust loader enforces address checks.
(() => {
  if (window === parent) return;
  const url = value => String(value).replace(/^https?:/i, scheme => "shina-" + scheme.toLowerCase());
  for (const [type, properties] of [
    [HTMLImageElement, ["src"]],
    [HTMLScriptElement, ["src"]],
    [HTMLLinkElement, ["href"]],
    [HTMLSourceElement, ["src"]],
    [HTMLMediaElement, ["src"]],
    [HTMLVideoElement, ["poster"]],
  ]) {
    for (const key of properties) {
      const descriptor = Object.getOwnPropertyDescriptor(type.prototype, key);
      if (descriptor && descriptor.set) {
        Object.defineProperty(type.prototype, key, {
          ...descriptor,
          set(value) { descriptor.set.call(this, url(value)); },
        });
      }
    }
  }
  const setAttribute = Element.prototype.setAttribute;
  Element.prototype.setAttribute = function(name, value) {
    return setAttribute.call(this, name, /^(src|href|poster)$/i.test(name) ? url(value) : value);
  };
})();
