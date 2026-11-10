import { View, div, svg } from "gpui-kit";
import { h_flex } from "gpui-base";
import { theme, layout, run, nextCommand, appHome, readTextFile, writeTextFile, notify } from "herdr";
import { stopsInit, stopsEnd, stop, focusRoot } from "./focus.js";
import { targetSpace, openAgentPicker, spaceDefaultAgent, agentShortLabel, agentLabel } from "./agents.js";

const TOOLS = [
  { id: "files", icon: "folder", label: "Files", node: "shina-files" },
  { id: "changes", icon: "git-branch", label: "Review changes", node: "shina-changes" },
  { id: "theme", icon: "sun", label: "Switch light or dark theme" },
];
function find(id, node = layout()) {
  if (node?.id === id) return node;
  for (const child of node?.children ?? []) {
    const hit = find(id, child);
    if (hit) return hit;
  }
  return null;
}

export default class ShinaTools extends View {
  init(_props, cx) {
    stopsInit(this, cx);
    cx.spawn(async (cx) => {
      for (;;) this.choose(await nextCommand(), cx);
    });
  }
  async close() { stopsEnd(this); }
  choose(id, cx) {
    if (id === "agent") return openAgentPicker("space");
    const tool = TOOLS.find(t => t.id === id);
    if (!tool) return;
    if (id === "theme") {
      if (this.changingTheme) return;
      this.changingTheme = true;
      const rgb = parseInt(theme().background.slice(1, 7), 16);
      const dark = ((rgb >> 16) * 0.299 + ((rgb >> 8) & 255) * 0.587 + (rgb & 255) * 0.114) < 128;
      cx.spawn(async () => {
        try {
          const file = `${appHome()}/settings.json`, before = await readTextFile(file);
          const next = JSON.parse(before ?? "{}");
          const currentDark = next.appearance === "dark" || next.appearance !== "light" && dark;
          next.appearance = currentDark ? "light" : "dark";
          await writeTextFile(file, JSON.stringify(next, null, 2) + "\n", before);
        } catch (e) {
          notify({ title: "Couldn't change theme", body: String(e), level: "error" });
        } finally { this.changingTheme = false; }
      });
    } else {
      const show = Boolean(find(tool.node)?.hidden);
      for (const other of TOOLS.filter(t => t.node && t.id !== id)) run("layout.hide", { id: other.node });
      run(show ? "layout.show" : "layout.hide", { id: tool.node });
    }
  }
  render() {
    const t = theme(), space = targetSpace(), agent = spaceDefaultAgent(space?.id);
    return focusRoot(this, h_flex().size_full().gap(4).items_center().justify_center().bg(t.chrome)
      .child(stop(this, t, "agent", h_flex().id("agent").h(28).px(8).gap(5).rounded(8).items_center().cursor_pointer()
        .role("button").accessibility_label("Default coding agent for this space")
        .tooltip(space ? `${space.name}: new threads use ${agentLabel(agent)}. Click to change the space default.` : "Choose a space first.")
        .text_color(t.muted).hover(e => e.bg(t.hover).text_color(t.foreground))
        .on_click((_e, cx) => this.choose("agent", cx))
        .child(svg(`icons/${agent}.svg`).size(13))
        .child(div().text_size(12).child(`Default · ${agentShortLabel(agent)}`)), 8))
      .children(TOOLS.map(tool => stop(this, t, tool.id, div().id(tool.id).size(28).rounded(8)
        .flex().items_center().justify_center().cursor_pointer()
        .role("button").accessibility_label(tool.label).tooltip(tool.label)
        .bg(tool.node && !find(tool.node)?.hidden ? t.selected : t.chrome)
        .text_color(t.muted).hover(e => e.bg(t.hover).text_color(t.foreground))
        .on_click((_e, cx) => this.choose(tool.id, cx))
        .child(svg(`mods/shina-tools/icons/${tool.icon}.svg`).size(15)), 8))));
  }
}
