import { View, div, image, svg } from "gpui-kit";
import { h_flex, v_flex } from "gpui-base";
import { theme, run, nextCommand, notify, openOverlay } from "herdr";
import { stopsInit, stopsEnd, stop, focusRoot } from "./focus.js";
import { targetSpace, spaceDefaultAgent, startThread } from "./agents.js";

const PROVIDERS = [
  { id: "pi", name: "Pi", detail: "Your configured models", icon: "pi" },
  { id: "claude", name: "Claude Code", detail: "Your existing Claude setup", icon: "claude" },
  { id: "codex", name: "Codex", detail: "Your existing Codex setup", icon: "codex" },
];

export default class ShinaHome extends View {
  init(_props, cx) {
    stopsInit(this, cx);
    cx.spawn(async () => {
      for (;;) {
        const command = await nextCommand();
        if (command === "terminal") this.terminal();
        else if (command === "new" || command.startsWith("new:")) this.open(command.split(":")[1]);
      }
    });
  }

  async close() { stopsEnd(this); }

  target() {
    return targetSpace();
  }

  open(id) {
    const s = this.target();
    if (!s) return run("mod.open", { name: "t3-new-menu" });
    if (s.machine) {
      notify({ title: "Remote chat", body: "This machine currently supports terminal sessions. Native remote chat is not connected yet.", level: "warning" });
      return;
    }
    startThread(s.id, id ?? spaceDefaultAgent(s.id));
  }

  terminal() {
    const s = this.target();
    if (s) run("tab.new", { space: s.id, folder: s.folder });
    else run("mod.open", { name: "t3-new-menu" });
  }

  render() {
    const t = theme(), s = this.target(), provider = spaceDefaultAgent(s?.id);
    const button = (id, label, callback, el) => stop(this, t, id, el.id(id).role("button")
      .accessibility_label(label).cursor_pointer().on_click(callback), 10);
    return focusRoot(this, v_flex().size_full().min_h(0).bg(t.canvas).px(24).pb(40).items_center().justify_center()
      .child(v_flex().w_full().max_w(580).gap(28)
        .child(v_flex().gap(12).items_center()
          .child(image("mods/shina-home/assets/ember.png").size(64))
          .child(div().font_family("Anthropic Serif Variable").font_normal().text_size(36).text_color(t.foreground).child("A little space to make things."))
          .child(div().text_size(14).text_color(t.muted).child(s ? `Start a thread in ${s.name}.` : "Add a project to start your first thread.")))
        .child(v_flex().gap(8).children(PROVIDERS.map(p =>
          button(`provider-${p.id}`, `New ${p.name} thread`, () => this.open(p.id),
            h_flex().w_full().min_h(64).px(18).py(12).gap(14).items_center().rounded(12)
              .border_1().border_color(provider === p.id ? t.accent + "88" : t.border).bg(t.surface)
              .hover(e => e.bg(t.hover))
              .child(svg(`icons/${p.icon}.svg`).size(22).text_color(t.accent))
              .child(v_flex().flex_1().min_w(0).gap(3)
                .child(div().text_size(14).font_medium().text_color(t.foreground).child(p.name))
                .child(div().text_size(12).text_color(t.muted).child(p.detail)))
              .child(div().text_size(14).text_color(t.dim).child("↗"))))))
        .child(h_flex().gap(12).justify_center()
          .child(button("new-project", "Add project", () => openOverlay({ mod: "t3-new-menu", kind: "popup", frame: "none", at: "top", offset: 96, width: 576, height: 420, props: { pick: true } }),
            div().px(10).py(6).rounded(8).text_size(12).text_color(t.muted).hover(e => e.bg(t.hover).text_color(t.foreground)).child("Add project")))
          .child(button("terminal", "Open terminal", () => this.terminal(),
            div().px(10).py(6).rounded(8).text_size(12).text_color(t.muted).hover(e => e.bg(t.hover).text_color(t.foreground)).child("Open terminal")))
          .child(button("find-thread", "Find thread", () => run("mod.open", { name: "t3-picker" }),
            div().px(10).py(6).rounded(8).text_size(12).text_color(t.muted).hover(e => e.bg(t.hover).text_color(t.foreground)).child("Find thread"))))));
  }
}
