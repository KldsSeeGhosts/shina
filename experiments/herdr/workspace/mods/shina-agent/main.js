import { View, div, svg } from "gpui-kit";
import { h_flex, v_flex } from "gpui-base";
import { props, theme, readTextFile, appHome, nextCommand, closeOverlay, run, notify, setShared } from "herdr";
import { AGENT_OPTIONS, agentLabel, targetSpace, spaceDefaultAgent, saveSpaceDefault, startThread, openAgentPicker } from "./agents.js";

export default class AgentPicker extends View {
  init(_props, cx) {
    this.options = props() ?? {};
    this.space = targetSpace(this.options.space);
    this.scope = this.options.scope ?? "new";
    this.current = this.scope === "thread" ? this.options.provider : spaceDefaultAgent(this.space?.id);
    this.index = Math.max(0, AGENT_OPTIONS.findIndex(a => a.id === this.current));
    this.handle = cx.focus_handle();
    this.available = null;
    cx.spawn(async (cx) => {
      try {
        const config = JSON.parse(await readTextFile(`${appHome()}/mods/chat/config.json`) ?? "{}");
        this.available = Object.fromEntries(AGENT_OPTIONS.map(a => [a.id, Boolean(config[a.id])]));
      } catch { this.available = {}; }
      this.handle.focus();
      this.snapshot();
      cx.notify();
    });
    cx.spawn(async (cx) => {
      for (;;) {
        const command = await nextCommand();
        if (command.startsWith("pick:")) this.choose(command.slice(5));
        else if (command === "snapshot") this.snapshot();
        cx.notify();
      }
    });
  }
  snapshot() {
    setShared("shina.agentPicker", { scope: this.scope, space: this.space?.id, current: this.current, available: this.available });
  }
  choose(id) {
    if (this.done || !this.available?.[id]) return;
    try {
      if (!targetSpace(this.space?.id)) throw new Error("This space is no longer open.");
      if (this.scope === "space") saveSpaceDefault(this.space.id, id);
      else if (this.scope === "thread") run(`chat.agent-select:${this.options.chat}:${id}`);
      else startThread(this.space.id, id);
      this.done = true;
      closeOverlay(this.options.overlay);
    } catch (e) {
      notify({ title: "Couldn't choose the coding agent", body: String(e), level: "error" });
    }
  }
  key(event, cx) {
    const key = event.key ?? event.keystroke;
    if (key === "escape") return closeOverlay(this.options.overlay);
    if (key === "enter") this.choose(AGENT_OPTIONS[this.index].id);
    else if (key === "up" || key === "down" || key === "tab") {
      const direction = key === "up" ? -1 : 1;
      for (let n = 0; n < AGENT_OPTIONS.length; n++) {
        this.index = (this.index + direction + AGENT_OPTIONS.length) % AGENT_OPTIONS.length;
        if (this.available?.[AGENT_OPTIONS[this.index].id]) break;
      }
    } else return;
    cx.stop_propagation();
    cx.notify();
  }
  render() {
    const t = theme(), space = this.space;
    const title = this.scope === "space" ? `Default agent for ${space?.name ?? "this space"}`
      : this.scope === "thread" ? "Coding agent for this thread" : `New thread in ${space?.name ?? "this space"}`;
    const description = this.scope === "space" ? "New threads use this agent. Existing threads stay unchanged."
      : this.scope === "thread" ? this.options.empty ? "Choose an agent. Your unsent draft and attachments stay with you."
        : `This conversation stays with ${agentLabel(this.current)}. Another agent opens a new thread.`
      : `Choose an agent for this thread. The space default is ${agentLabel(this.current)}.`;
    return v_flex().size_full().p(8).gap(4).rounded(14).border_1().border_color(t.border).bg(t.popover).shadow_lg().text_color(t.foreground)
      .track_focus(this.handle).on_key_down((e, cx) => this.key(e, cx))
      .child(v_flex().px(10).pt(8).pb(10).gap(6)
        .child(div().text_size(15).font_semibold().child(title))
        .child(div().text_size(12).line_height("18px").text_color(t.muted).child(description)))
      .children(AGENT_OPTIONS.map((agent, index) => {
        const available = this.available?.[agent.id], selected = agent.id === this.current;
        return h_flex().id(`agent-${agent.id}`).h(42).px(10).gap(10).items_center().rounded(8)
          .role("button").accessibility_label(agent.name).text_size(13)
          .when(index === this.index, e => e.bg(t.selected))
          .when(!available, e => e.opacity(0.5))
          .when(Boolean(available), e => e.cursor_pointer().hover(s => s.bg(t.hover)).on_click(() => this.choose(agent.id)))
          .child(svg(`icons/${agent.id}.svg`).size(18).flex_none().text_color(t.accent))
          .child(div().flex_1().child(this.scope === "thread" && !this.options.empty && !selected ? `New ${agent.name} thread` : agent.name))
          .child(div().text_size(11).text_color(t.muted).child(!this.available ? "Checking…" : !available ? "Not installed" : selected ? "Selected" : ""));
      }))
      .when(this.scope === "thread", e => e.child(div().id("space-default").mt(4).px(10).py(10).rounded(8)
        .text_size(12).text_color(t.muted).cursor_pointer().hover(s => s.bg(t.hover).text_color(t.foreground))
        .role("button").accessibility_label("Choose the default agent for this space")
        .on_click(() => { closeOverlay(this.options.overlay); openAgentPicker("space", { space: space.id }); })
        .child(`Space default: ${agentLabel(spaceDefaultAgent(space?.id))}`)))
      .child(div().px(10).pt(4).pb(6).text_size(11).text_color(t.dim).child("↑ ↓ to navigate · Enter to choose · Esc to close"));
  }
}
