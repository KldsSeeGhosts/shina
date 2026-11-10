import { View, div, image, svg } from "gpui-kit";
import { Button, h_flex, v_flex, Textarea, TextareaState, TextView } from "gpui-base";
import { spaces, agents, focused, props, theme, settings, run, shared, setShared, nextCommand, appHome, readTextFile, writeTextFile, exec } from "herdr";

const CAPTAINS = [
  { key: "ember", name: "Ember", role: "Platform" },
  { key: "juniper", name: "Juniper", role: "Release" },
  { key: "sable", name: "Sable", role: "Support" },
  { key: "pip", name: "Pip", role: "Research" },
];
const ASSETS = "mods/shina-native/assets/";
const SERIF = "Anthropic Serif Variable Text";
const storeKey = (tab) => `shina.thread.${tab}`;
const currentSpace = () => spaces().find((space) => space.active);
const metadata = (tab) => shared(storeKey(shared(`shina.tab.${tab}`) ?? tab)) ?? {};
const label = (text, size, color) => div().text_size(size).text_color(color).child(text);
const spacer = () => div().flex_1().min_w(0);
const icon = (name, color, size = 16) => svg(`${ASSETS}${name}.svg`).size(size).flex_none().text_color(color);
const portrait = (key, size = 22) => image(`${ASSETS}${key}.png`).size(size).flex_none().rounded(5);
const control = (id, name, action) => {
  const t = theme();
  return Button.new(id).accessibility_label(name).tooltip(name).h(28).px(8).rounded(7)
    .text_size(12).text_color(t.muted).hover((s) => s.bg(t.hover).text_color(t.foreground))
    .on_mouse_down("left", (_e, cx) => cx.stop_propagation()).on_click(action);
};
const heading = (name) => h_flex().h(36).pt(8).px(10).items_center()
  .child(label(name, 12, theme().muted));

function newThread(captain = null) {
  const space = currentSpace();
  if (!space) return;
  const name = captain ? `Captain ${CAPTAINS.find((c) => c.key === captain).name}` : "New thread";
  const existing = space.tabs.find((tab) =>
    metadata(tab.id).captain === captain && metadata(tab.id).empty
    || !metadata(tab.id).created && tab.name === name);
  if (existing) return run("tab.select", { id: existing.id });
  const threadId = `thread-${Date.now()}-${Math.random().toString(36).slice(2, 9)}`;
  run("tab.new", { space: space.id, name, mod: "shina-native", props: { role: "chat", captain, threadId } });
}

export default class ShinaNative extends View {
  init(_props, cx) {
    this.role = props()?.role ?? "chat";
    this.handle = cx.focus_handle();
    this.expanded = false;
    this.width = 800;
    this.filter = "active";
    this.error = "";
    this.probing = false;
    this.probeBlocked = false;
    this.probe = null;
    if (this.role === "chat") {
      // Herdr keeps inactive tab mods alive. A reload must not bind every copy
      // to whichever tab happens to be focused at that moment.
      this.thread = props()?.threadId ?? shared("shina.legacyThread") ?? "legacy-thread";
      this.data = shared(storeKey(this.thread)) ?? {};
      this.tab = this.data.tab ?? null;
      if (!this.data.created) {
        this.data = {
          created: Date.now(), captain: props()?.captain ?? null,
          empty: true, draft: "", messages: [], example: false,
        };
      }
      if (!props()?.threadId) setShared("shina.legacyThread", this.thread);
      this.save();
      this.input = TextareaState.new({ placeholder: "Describe a task or ask a question...", value: this.data.draft ?? "", rows: 2 });
      this.input.set_auto_grow(2, 6);
      this.input.on("change", (_e, cx) => {
        this.bindOwner();
        this.data.draft = this.input.value();
        this.save();
        cx.notify();
      });
      this.input.on("submit", (event, cx) => {
        if (!event.shift) this.submit(cx);
      });
      this.input.on("focus", (_e, cx) => { this.bindOwner(); this.inputFocused = true; cx.notify(); });
      this.input.on("blur", (_e, cx) => { this.inputFocused = false; cx.notify(); });
    }
    cx.spawn(async (cx) => {
      for (;;) {
        const command = await nextCommand();
        if (this.role === "header" && command === "theme") this.switchTheme(cx);
        // Herdr delivers commands only to shown copies. Its focused snapshot is
        // current here, unlike during construction of a newly allocated tab.
        if (this.role !== "chat") continue;
        this.bindOwner();
        if (command === "focus") this.input.focus();
        if (command === "example") this.loadExample(cx);
        if (command === "submit-demo") {
          this.input.set_value("Show me what we can keep from Shina in this native Herdr port.");
          this.submit(cx);
        }
        if (command === "toggle-work") { this.expanded = !this.expanded; cx.notify(); }
        if (command === "new-thread") newThread();
        if (command === "draft-demo") {
          this.input.set_value("A draft kept while switching native Herdr tabs.");
          this.data.draft = this.input.value();
          this.save();
          cx.notify();
        }
        if (command === "probe-pi") this.probePi(cx);
        if (command === "snapshot") setShared("shina.snapshot", {
          snapshotId: `${Date.now()}-${Math.random()}`,
          tab: this.tab, thread: this.thread, draft: this.input.value(),
          messages: this.data.messages, probe: this.probe, error: this.error, focused: Boolean(this.inputFocused),
        });
      }
    });
  }

  save() { setShared(storeKey(this.thread), this.data); }

  bindOwner() {
    const tab = focused()?.tab;
    if (!tab || this.tab && this.tab !== tab) return;
    this.tab = tab;
    this.data.tab = tab;
    setShared(`shina.tab.${tab}`, this.thread);
    this.save();
  }

  switchTheme(cx) {
    cx.spawn(async (cx) => {
      try {
        const path = `${appHome()}/settings.json`;
        const previous = await readTextFile(path);
        const next = JSON.parse(previous ?? "{}");
        const dark = next.appearance === "dark" || next.appearance !== "light" && settings().dark;
        next.appearance = dark ? "light" : "dark";
        await writeTextFile(path, JSON.stringify(next, null, 2) + "\n", previous);
      } catch (error) {
        this.error = String(error);
      }
      cx.notify();
    });
  }

  probePi(cx) {
    if (this.probing || this.probeBlocked) return;
    this.probing = true;
    this.error = "";
    cx.notify();
    cx.spawn(async (cx) => {
      try {
        const config = JSON.parse(await readTextFile(`${appHome()}/shina-runtime.json`) ?? "{}");
        if (!config.node || !config.probe) throw new Error("Run the prototype setup command first.");
        const result = await exec(config.node, [config.probe, "--cwd", config.workspace], { timeout: 25_000 });
        if (result.code !== 0) throw new Error(result.stderr.trim() || "Pi discovery failed.");
        this.probe = JSON.parse(result.stdout);
      } catch (error) {
        this.error = String(error);
        this.probeBlocked = this.error.includes("authentication failed");
      }
      this.probing = false;
      cx.notify();
    });
  }

  submit(cx) {
    this.bindOwner();
    const text = this.input.value().trim();
    if (!text) return;
    this.data.messages.push({ role: "user", text });
    this.data.messages.push({
      role: "assistant",
      text: "This is a **local prototype response**, not an AI run.\n\nThe composer, transcript, captain portraits, sidebar, and typography now render as native GPUI elements inside Herdr. These are real Herdr tabs, so switching threads preserves the draft and conversation.\n\nThe next integration is Pi's structured RPC stream. No model request has been sent.",
    });
    this.data.empty = false;
    this.data.example = true;
    this.data.draft = "";
    this.input.set_value("");
    this.save();
    run("tab.rename", { id: this.tab, name: text.slice(0, 36) });
    cx.notify();
  }

  loadExample(cx) {
    this.bindOwner();
    this.data.messages = [
      { role: "user", text: "Adapt Shina's current design into Herdr's native framework." },
      { role: "assistant", text: "I'll keep the composer and thread hierarchy, and let Herdr manage the window, tabs, and terminals." },
      { role: "work", text: "Native port checks", tools: [
        ["Interface", "Native GPUI elements. No embedded browser or Rune window."],
        ["Threads", "Herdr owns the spaces and tabs. Shina keeps conversation state and drafts."],
        ["Typography", "Installed Anthropic Sans and Serif for controls and reading."],
      ] },
      { role: "assistant", text: "The first native pass is in place.\n\nThe sidebar keeps Shina's captains and thread titles. The composer has its own multiline input, and replies use the same serif reading style as our standalone app.\n\n**What still needs proving**\n\nPi's structured events, approvals, and attachment handling need an adapter. The existing mobile app sees Herdr terminals, but it does not automatically inherit this desktop mod." },
    ];
    this.data.empty = false;
    this.data.example = true;
    this.save();
    run("tab.rename", { id: this.tab, name: "Native port of Shina" });
    cx.notify();
  }

  header() {
    const t = theme();
    const shown = currentSpace()?.tabs.filter((tab) => !tab.settled) ?? [];
    return h_flex().size_full().items_center().bg(t.chrome).gap(5).pr(10)
      .child(h_flex().w(258).flex_none().pl(94).gap(8).items_center()
        .child(div().size(22).rounded(6).bg(t.foreground).text_color(t.chrome).font_semibold().flex().items_center().justify_center().child("S"))
        .child(label("Shina", 13, t.foreground))
        .child(spacer())
        .child(control("shina-collapse", "Toggle sidebar", () => run("layout.toggle", { id: "sidebar" })).child(icon("sidebar", t.muted))))
      .child(h_flex().id("shina-tabs").flex_1().min_w(0).gap(5).overflow_x_scrollbar().children(shown.map((tab) =>
        Button.new(`shina-tab-${tab.id}`).accessibility_label(tab.name).selected(tab.active)
          .h(29).px(10).max_w(260).rounded(7).bg(tab.active ? t.canvas : t.chrome)
          .text_size(12).text_color(tab.active ? t.foreground : t.muted)
          .hover((s) => s.bg(t.hover)).on_mouse_down("left", (_e, cx) => cx.stop_propagation())
          .on_click(() => run("tab.select", { id: tab.id }))
          .child(h_flex().gap(8).items_center()
            .child(label("shina /", 12, t.muted))
            .child(div().min_w(0).truncate().child(tab.name))))))
      .child(control("shina-new-tab", "New thread", () => newThread()).child(icon("plus", t.muted)));
  }

  sidebar() {
    const t = theme();
    const space = currentSpace();
    const threads = space?.tabs ?? [];
    const live = agents();
    const nav = (id, title, glyph, action, active = false) =>
      control(id, title, action).h(34).w_full().px(10).bg(active ? t.selected : t.chrome)
        .child(h_flex().w_full().gap(9).items_center().child(icon(glyph, t.muted)).child(label(title, 14, t.foreground)));
    const threadRow = (tab) => Button.new(`shina-row-${tab.id}`).accessibility_label(tab.name).selected(tab.active)
      .w_full().px(10).py(8).rounded(8).bg(tab.active ? t.selected : t.chrome)
      .hover((s) => s.bg(t.hover)).on_click(() => run("tab.select", { id: tab.id }))
      .child(v_flex().w_full().gap(4)
        .child(h_flex().gap(7).items_center().child(icon("thread", t.muted, 13))
          .child(label(space?.name ?? "No project", 12, t.muted)))
        .child(div().w_full().truncate().text_size(14).text_color(t.foreground).font_weight(tab.active ? 550 : 420).child(tab.name)));
    return v_flex().size_full().pb(8).gap(2).text_color(t.foreground).bg(t.chrome)
      .child(nav("shina-new", "New thread", "compose", () => newThread(), true))
      .child(nav("shina-terminal", "Open terminal", "terminal", () =>
        run("tab.new", { space: space?.id, name: "Terminal", folder: space?.folder })))
      .child(heading("Captains"))
      .children(CAPTAINS.map((captain) =>
        Button.new(`captain-${captain.key}`).w_full().px(10).py(8).rounded(8)
          .accessibility_label(`Open Captain ${captain.name}`).hover((s) => s.bg(t.hover))
          .on_click(() => {
            const tab = threads.find((tab) => metadata(tab.id).captain === captain.key);
            if (tab) run("tab.select", { id: tab.id }); else newThread(captain.key);
          }).child(h_flex().w_full().gap(9).items_center().child(portrait(captain.key))
            .child(v_flex().gap(3).child(label(captain.name, 14, t.foreground))
              .child(label(captain.role, 12, t.muted))))))
      .child(heading("Threads").child(spacer())
        .child(control("shina-add", "New thread", () => newThread()).child(icon("plus", t.muted, 14))))
      .child(h_flex().h(24).px(10).items_center().child(label(space?.name ?? "Shina prototype", 12, t.muted)))
      .child(v_flex().id("shina-thread-scroll").flex_1().min_h(0).gap(3).overflow_y_scrollbar()
        .children(threads.filter((tab) => !tab.settled).map(threadRow))
        .when(threads.some((tab) => tab.settled), (el) =>
          el.child(heading("Settled")).children(threads.filter((tab) => tab.settled).map(threadRow))))
      .child(div().h(1).mx(8).bg(t.border))
      .child(h_flex().h(44).px(10).items_center()
        .child(v_flex().gap(3).child(label("Shina on Herdr", 12, t.foreground))
          .child(label(`${live.length} live agents · native prototype`, 11, t.muted)))
        .child(spacer())
        .child(control("shina-theme", "Switch theme", () =>
          run("shina-native.theme")).child(icon("theme", t.muted))));
  }

  work(message) {
    const t = theme();
    return v_flex().gap(8)
      .child(control("shina-work-toggle", `${this.expanded ? "Hide" : "Show"} native port checks`, (_e, cx) => {
        this.expanded = !this.expanded; cx.notify();
      }).w_full().px(0).child(h_flex().w_full().gap(10).items_center()
        .child(div().h(1).flex_1().bg(t.border))
        .child(label(message.text, 12, t.muted))
        .child(icon(this.expanded ? "down" : "right", t.muted, 12))
        .child(div().h(1).flex_1().bg(t.border))))
      .child(v_flex().id("shina-work-detail").h(this.expanded ? 150 : 0).overflow_hidden()
        .spring("height", { response: 220, damping: 1 })
        .children(message.tools.map(([title, text]) => h_flex().gap(10).py(6).items_start()
          .child(label("✓", 12, t.accent))
          .child(v_flex().gap(3).child(label(title, 12, t.foreground))
            .child(label(text, 12, t.muted))))));
  }

  chat(cx) {
    const t = theme();
    const captain = CAPTAINS.find((captain) => captain.key === this.data.captain) ?? CAPTAINS[0];
    const empty = this.data.messages.length === 0;
    const narrow = this.width < 560;
    const measure = this.width >= 1200 ? 880 : 768;
    const bubbleWidth = Math.max(100, Math.min(620, Math.min(measure, this.width) - (narrow ? 32 : 48)));
    const prose = (message, index) => {
      if (message.role === "work") return this.work(message);
      if (message.role === "user") return h_flex().w_full().justify_end().child(
        div().max_w(bubbleWidth).min_w(0).flex_shrink(1).px(14).py(10).rounded(12)
          .bg(t.selected).text_size(14).text_color(t.foreground).child(message.text));
      return TextView.markdown(`shina-message-${this.thread}-${index}`, message.text)
        .selectable(true).scrollable(false).font_family(SERIF).text_size(16).line_height("25px").text_color(t.foreground).w_full();
    };
    const intro = v_flex().gap(18).pb(10)
      .child(h_flex().gap(12).items_center()
        .when(Boolean(this.data.captain), (el) => el.child(portrait(captain.key, 34)))
        .child(label(this.data.captain ? `Captain ${captain.name}` : "What are we working on?", 30, t.foreground)
          .font_family(SERIF).line_height("38px")))
      .child(label(this.data.captain ? `${captain.role} · Shina's native Herdr prototype` : "Shina's design, rendered natively by Herdr.", 13, t.muted))
      .child(h_flex().gap(8).flex_wrap()
        .child(control("shina-example", "Show example conversation", (_e, cx) => this.loadExample(cx))
          .border(1).border_color(t.border).rounded(15).child("Show example"))
        .child(control("shina-core-terminal", "Open a real Herdr terminal", () =>
          run("tab.new", { space: currentSpace()?.id, name: "Terminal", folder: currentSpace()?.folder }))
          .border(1).border_color(t.border).rounded(15).child("Open terminal")));
    const composer = v_flex().w_full().max_w(measure).px(narrow ? 16 : 24).pb(16).gap(8)
      .when(Boolean(this.error), (el) => el.child(label(this.error, 12, t.blocked)))
      .child(v_flex().w_full().rounded(16).border(1).border_color(this.inputFocused ? t.input : t.border)
        .bg(t.background).px(16).pt(14).pb(12).gap(12)
        .child(Textarea.new(this.input).accessibility_label("Message in Shina prototype")
          .w_full().h(58).text_size(15).text_color(t.foreground).bg(t.background))
        .child(h_flex().gap(6).items_center()
          .child(label("Local demo", 12, t.muted))
          .child(spacer())
          .child(control("shina-probe", "Read Pi model metadata without sending a prompt", (_e, cx) => this.probePi(cx))
            .disabled(this.probing || this.probeBlocked).max_w(narrow ? 160 : 260)
            .child(div().truncate().child(this.probing ? "Reading Pi..." : this.probe ? `Pi found · ${this.probe.model}` : "Probe Pi connection")))
          .child(control("shina-send", "Send local demo message", (_e, cx) => this.submit(cx))
            .size(32).px(0).bg(this.input.value().trim() ? t.accent : t.raised)
            .text_color(this.input.value().trim() ? t.accent_foreground : t.muted)
            .disabled(!this.input.value().trim()).child(icon("send", this.input.value().trim() ? t.accent_foreground : t.muted)))))
      .child(h_flex().gap(7).items_center().flex_wrap()
        .child(icon("folder", t.muted, 13))
        .child(label(`${currentSpace()?.name ?? "Shina"} · local`, 12, t.muted))
        .child(spacer()).child(label("Prototype only. No AI request is sent.", 11, t.muted)));
    return v_flex().id("shina-chat-root").size_full().min_w(0).min_h(0).bg(t.canvas).text_color(t.foreground)
      .track_focus(this.handle)
      .on_size_change((size, cx) => { if (this.width !== size.width) { this.width = size.width; cx.notify(); } })
      .child(h_flex().h(38).flex_none().px(16).gap(8).items_center()
        .child(spacer())
        .child(label(this.data.example ? "Example conversation" : "Native prototype", 11, t.muted))
        .child(control("shina-settle", "Settle this thread", () => run("tab.settle", { id: focused()?.tab }))
          .child(icon("check", t.muted, 14))))
      .when(empty, (el) => el.child(spacer()))
      .child(v_flex().id(`shina-transcript-${this.thread}`).w_full().min_h(0)
        .when(!empty, (el) => el.flex_1().overflow_y_scrollbar())
        .items_center().child(v_flex().w_full().max_w(measure).px(narrow ? 16 : 24).py(20).gap(22)
          .when(empty, (el) => el.child(intro))
          .when(!empty, (el) => el.child(h_flex().gap(8).items_center()
            .child(portrait(captain.key, 20)).child(label(`Captain ${captain.name}`, 13, t.foreground)))
            .children(this.data.messages.map(prose)))))
      .child(h_flex().w_full().justify_center().child(composer))
      .when(empty, (el) => el.child(spacer()));
  }

  render(cx) {
    if (this.role === "header") return this.header();
    if (this.role === "sidebar") return this.sidebar();
    return this.chat(cx);
  }
}
