# Third-party notices

Shina's Herdr workspace adapts these projects. The exact commits are in
`upstreams.json`. The installer retains the upstream checkouts and copies their
licenses into the private profile's `licenses/` directory.

- [herdr-chat](https://github.com/ChmaraX/herdr-chat), copyright 2026 Adam Chmara,
  MIT. Shina modifies the chat presentation, draft handling, send acknowledgment,
  launcher, keyboard defaults, and private-core launch environment.
- [herdr-t3](https://github.com/JinJieBeWater/herdr-t3), Apache-2.0. Shina modifies
  the layout, branding, new-thread routing, machine portraits, and subagent-chat
  navigation. Its mods originally adapt Herdr App's shipped mods.
- [Herdr App](https://github.com/herdrdev/herdr-app), Apache-2.0. Shina uses its
  installed Files and Changes panels without modifying them.
- [Lucide](https://lucide.dev), copyright Lucide Contributors, ISC. The T3 mods
  include Lucide icons. Herdr's installed icon notices remain in `icons/`.
- Herdr supplies the agent logos and their notices in its installed profile.

Anthropic font files do not ship with this repository. The optional font step
reads the user's installed fonts and produces static instances inside the private
profile. It preserves the source fonts' copyright and license name records.
Do not redistribute those generated files without permission from their owner.

Shina's own crew artwork and open-licensed fallback font assets retain their
existing repository ownership and licenses.
