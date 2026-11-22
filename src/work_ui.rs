//! Expanded work content ported from KSG Code's V2ItemInspector and ToolCallBody.
use super::*;
use crate::work::ToolKind;
use serde_json::Value;
#[cfg(test)]
#[path = "work_ui_tests.rs"]
mod tests;

pub(super) fn boundary_events(s: &State, p: Palette, wide: bool, message: usize) -> Option<El> {
    let structure = timeline_structure(s.current());
    let events = structure.boundaries.get(&message)?;
    let mut rows = col().min_w(0.0).gap(6.0);
    for &(_, kind, index) in events {
        rows = rows.child(match kind {
            0 => pi_ui::compaction_activity(s, p, index),
            1 => retry_row(s, p, index),
            _ => assistant_turn(s, p, wide, message, message),
        });
    }
    Some(rows)
}

/// KSG's providerRetryWorkLogSummary, kept as one replaceable inline event.
pub(super) fn retry_row(s: &State, p: Palette, index: usize) -> El {
    let event = &crate::work::retry_events(s.current())[index];
    let title = if event["running"] == true {
        let attempt = event["attempt"].as_u64().unwrap_or(1);
        let progress = event["maxAttempts"]
            .as_u64()
            .map_or_else(|| attempt.to_string(), |max| format!("{attempt}/{max}"));
        format!("Retrying provider ({progress})")
    } else if event["success"] == true {
        "Provider recovered".into()
    } else {
        event["error"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or("Provider error")
            .into()
    };
    row()
        .key(format!("provider-retry-{}-{index}", s.current().id))
        .min_w(0.0)
        .role("status")
        .min_h(26.0)
        .gap(5.0)
        .center_y()
        .child(el().size(24.0, 24.0).center().child(icon("clock", p.muted)))
        .child(if event["running"] == true || event["success"] == true {
            meta_text(title, p.muted)
                .min_w(0.0)
                .shrink()
                .wrap_anywhere()
        } else {
            label(title, SMALL, p.muted)
                .min_w(0.0)
                .shrink()
                .wrap_anywhere()
        })
}

/// Ported from KSG Code's `workEntryIconName`/`toolGroupSummaryIconName`
/// (`TimelineToolActivityIcon.tsx`, `TimelineWorkEntryRow.logic.ts`).
pub(super) fn kind_icon(kind: ToolKind) -> &'static str {
    match kind {
        ToolKind::Command => "terminal",
        ToolKind::FileChange => "square-pen",
        ToolKind::FileSearch => "search",
        ToolKind::WebSearch => "globe",
        ToolKind::Read => "eye",
        ToolKind::List => "folder",
        ToolKind::Subagent => "bot",
        ToolKind::Plan => "list-todo",
        ToolKind::Html => "open",
        ToolKind::Other => "wrench",
    }
}

fn mono_style() -> TextStyle {
    TextStyle::new(12.0, 400.0)
        .family(rune::Family::SystemMono)
        .lh(18.0)
}

/// KSG's `--font-size-code` mono at `text-foreground/85`, for commands, patterns and
/// argument values (`ToolCallBody`, `ShellCommandBlock`).
fn mono(value: impl Into<String>, p: Palette) -> El {
    label(value, mono_style(), p.ink.alpha(0.85))
        .min_w(0.0)
        .wrap_anywhere()
        .selectable(true)
}

/// KSG's mono at `text-dim-foreground`, for changed-file lists and search result paths.
fn mono_dim(value: impl Into<String>, p: Palette) -> El {
    label(value, mono_style(), p.faint)
        .min_w(0.0)
        .wrap_anywhere()
        .selectable(true)
}

/// Ported from KSG Code's `WORK_LOG_OUTPUT_CLASS_NAME`: quiet mono tool output.
fn mono_muted(value: impl Into<String>, p: Palette) -> El {
    label(value, mono_style(), p.muted)
        .min_w(0.0)
        .wrap_anywhere()
        .selectable(true)
}

/// A radius-8 `surface-well` frame used only for embedded commands, diffs, and raw payloads.
fn code_well(p: Palette) -> El {
    col()
        .min_w(0.0)
        .radius(8.0)
        .clip_rounded(8.0)
        .fill(p.code_bg)
        .border(1.0, p.line)
}

/// KSG's `StructuredValue` bound (`max-h-80`, 320 px).
fn block(key: String, content: El, running: bool) -> El {
    col()
        .key(key)
        .min_w(0.0)
        .max_h(320.0)
        .scroll(ScrollSpec {
            stick_bottom: running,
        })
        .child(content)
}

pub(super) fn relative(path: &str, root: &std::path::Path) -> String {
    crate::work::relative_path(path, root)
}

fn padded(content: El) -> El {
    content.px(12.0).py(9.0)
}

/// KSG's `isJsonToolText`: true when the string parses as a JSON object or array.
fn looks_like_json(text: &str) -> bool {
    let trimmed = text.trim_start();
    (trimmed.starts_with('{') || trimmed.starts_with('['))
        && serde_json::from_str::<Value>(trimmed).is_ok()
}

/// KSG's `canExpandProjectedItem` proxy: the row offers a disclosure only when its body
/// has something beyond the collapsed label.
pub(super) fn has_body(tool: &Tool) -> bool {
    tool.expanded
        || !tool.output.is_empty()
        || !tool.input.is_null()
        || !tool.changes.is_empty()
        || !tool.images.is_empty()
        || tool.exit_code.is_some()
        || tool.parent_call.is_some()
        || !tool.structured.is_null()
        || !tool.detail.is_empty()
}

pub(super) fn input_text(tool: &Tool, keys: &[&str]) -> String {
    if let Some(input) = tool.input.as_str() {
        return input.into();
    }
    for input in [
        &tool.input,
        &tool.input["action"],
        &tool.structured["input"],
    ] {
        for key in keys {
            if let Some(value) = input.get(*key) {
                if let Some(value) = value.as_str() {
                    return value.into();
                }
                if let Some(values) = value.as_array() {
                    return values
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join("\n");
                }
            }
        }
    }
    tool.detail.clone()
}

fn output(tool: &Tool, p: Palette, id: &str) -> El {
    let text = if tool.output.is_empty() {
        if tool.done {
            "No output"
        } else {
            "Waiting for output"
        }
    } else {
        &tool.output
    };
    block(
        format!("tool-text-output-{id}"),
        mono_muted(text, p).py(2.0),
        !tool.done,
    )
}

/// Ported from KSG Code's reasoning-only group summary. Keep one disclosure per
/// consecutive run of thoughts, with the original text keys inside it.
pub(super) fn thoughts(s: &State, p: Palette, indices: &[usize], listed: bool) -> El {
    let t = s.current();
    let mut content = col()
        .min_w(0.0)
        .gap(4.0)
        .child(thought_head(s, p, indices, listed));
    if t.thoughts[indices[0]].expanded {
        for index in indices {
            content = content.child(thought_body(s, p, *index));
        }
    }
    content
}

/// The disclosure line of a thought run. Long turns place it and each body in
/// their own transcript slice; `thoughts` keeps them together for short turns.
/// Ported from KSG Code's `WorkEntryLogRow` reasoning labels: a folded single thought
/// reads the thought's own text, an unfolded one reads "Thought" ("Thinking" while live),
/// and a thought run's head reads "Thought (×N)", the icon the brain glyph.
pub(super) fn thought_head(s: &State, p: Palette, indices: &[usize], listed: bool) -> El {
    let t = s.current();
    let first = indices[0];
    let expanded = t.thoughts[first].expanded;
    let block = t.thoughts[first].block;
    let offset = t.thoughts[first].anchor.offset;
    let anchor_msg_id = t
        .messages
        .get(t.thoughts[first].anchor.message)
        .map(|m| m.id.clone());
    let key = format!("inline-thought-{}-{first}", t.id);
    let press_key = key.clone();
    let title = if indices.len() == 1 {
        if expanded {
            if t.thoughts[first].done {
                "Thought".into()
            } else {
                "Thinking".into()
            }
        } else {
            let line = live_thought_line(&t.thoughts[first].text);
            if line.is_empty() {
                "Thought".into()
            } else {
                line
            }
        }
    } else {
        format!("Thought (×{})", indices.len())
    };
    // KSG offers the disclosure only when the thought has text to show.
    let expandable = indices.len() > 1 || !t.thoughts[first].text.trim().is_empty();
    activity_line(
        key,
        format!("{} thinking text", if expanded { "Hide" } else { "Show" }),
        title,
        None,
        false,
        false,
        expanded,
        expandable,
        listed,
        "brain",
        p,
    )
    .on_press_tx(move |s: &mut State, tx| {
        tx.keep_on_screen(press_key.clone());
        let t = s.current_mut();
        let msg_idx = anchor_msg_id
            .as_deref()
            .and_then(|id| t.messages.iter().position(|m| m.id == id));
        if anchor_msg_id.is_some() && msg_idx.is_none() {
            return;
        }
        if let Some(thought) = t.thoughts.iter_mut().find(|th| {
            th.block == block
                && th.anchor.offset == offset
                && msg_idx.is_none_or(|idx| th.anchor.message == idx)
        }) {
            thought.expanded = !thought.expanded;
            let message = thought.anchor.message;
            t.timeline_cache.get_mut().touch_message(message);
        }
    })
}

/// KSG's `WorkLogDetails` for reasoning: 30 pt in (`ms-7.5`), one bounded scroll region
/// (`max-h-96`), the thinking rendered as markdown in the foreground colour
/// (`ReasoningTraceContent`).
pub(super) fn thought_body(s: &State, p: Palette, index: usize) -> El {
    let t = s.current();
    let thought = &t.thoughts[index];
    let empty = thought.text.is_empty();
    let text = if empty {
        "The provider did not provide visible thinking text.".to_owned()
    } else {
        thought.text.clone()
    };
    col()
        .key(format!("thought-output-{}-{index}", t.id))
        .role("group")
        .label("Thinking text")
        .min_w(0.0)
        .pl(30.0)
        .mb(4.0)
        .py(4.0)
        .max_h(384.0)
        .scroll(ScrollSpec {
            stick_bottom: !thought.done,
        })
        .child(if empty {
            label(text, BODY, p.muted).wrap_anywhere().selectable(true)
        } else {
            markdown_ui::render_with_find(
                &text,
                p,
                s.dark,
                false,
                false,
                !thought.done,
                &format!("thought-{}-{index}", t.id),
                &s.unwrapped_code_blocks,
                None,
            )
        })
}

/// Ported from KSG Code's `WorkLogDetails` (`ms-7.5` indent, `max-h-96` scroll region,
/// `gap-3` between sections, `py-1`) and `V2ItemInspector` (`ToolCallBody` and the
/// file_change, file_search and web_search sections).
pub(super) fn expanded(s: &State, p: Palette, index: usize) -> El {
    let t = s.current();
    let tool = &t.tools[index];
    let id = format!("{}-{index}", t.id);
    let root = s.working_directory();
    let kind = tool.resolved_kind();
    let mut body = col()
        .key(format!("tool-output-{id}"))
        .role("group")
        .label(format!("Output for {}", tool_title(tool, &root)))
        .min_w(0.0)
        .mb(6.0)
        .ml(30.0)
        .py(4.0)
        .gap(12.0)
        .max_h(384.0)
        .scroll(ScrollSpec {
            stick_bottom: !tool.done,
        });
    match kind {
        ToolKind::Command => {
            // KSG's ToolCallBody: the command at text-foreground/85, muted output, then
            // `exit {code}` in destructive. No wells or labels.
            let mut section = col().min_w(0.0).gap(6.0).child(
                mono(input_text(tool, &["command", "cmd"]), p).key(format!("tool-command-{id}")),
            );
            section = section.child(output(tool, p, &id));
            body = body.child(section);
        }
        ToolKind::FileChange if !tool.changes.is_empty() => {
            // KSG's file_change inspector: counts and an "Open diff" action on one line,
            // then the changed paths in quiet mono. Diffs stay in the diff panel.
            let additions: usize = tool.changes.iter().map(|c| c.additions).sum();
            let deletions: usize = tool.changes.iter().map(|c| c.deletions).sum();
            let first_path = tool.changes[0].path.clone();
            let mut head = row().center_y().gap(12.0);
            if additions > 0 {
                head = head.child(label(format!("+{additions}"), SMALL, p.ok));
            }
            if deletions > 0 {
                head = head.child(label(format!("-{deletions}"), SMALL, p.danger));
            }
            head = head.child(crew_ui::ghost_button(
                s,
                format!("tool-open-diff-{id}"),
                "Open diff",
                p,
                move |s: &mut State| {
                    let panel_key = s.panel_key();
                    crate::ui::diff_panel::reveal_file(s, &panel_key, &first_path);
                },
            ));
            body = body.child(head);
            if tool.failed && !tool.output.is_empty() {
                body = body.child(output(tool, p, &id));
            }
            let mut list = col().min_w(0.0).gap(4.0);
            for (file, change) in tool.changes.iter().enumerate() {
                let mut line = change.operation.clone();
                if let Some(old) = &change.old_path {
                    line.push_str(&format!(" {} →", relative(old, &root)));
                }
                line.push_str(&format!(" {}", relative(&change.path, &root)));
                let path = change.path.clone();
                list = list.child(
                    mono_dim(line, p)
                        .key(format!("tool-file-path-{id}-{file}"))
                        .role("button")
                        .label(format!("Open diff for {}", relative(&change.path, &root)))
                        .cursor(rune::Cursor::Pointer)
                        .focusable(true)
                        .w_full()
                        .radius(6.0)
                        .hover(p.selected)
                        .on_press(move |s: &mut State| {
                            let panel_key = s.panel_key();
                            crate::ui::diff_panel::reveal_file(s, &panel_key, &path);
                        }),
                );
            }
            body = body.child(list);
        }
        ToolKind::FileSearch => {
            // KSG's file_search inspector: the pattern at text-foreground/85, then a
            // path list in dim mono with a sans preview under each entry.
            body = body.child(
                mono(input_text(tool, &["pattern", "query"]), p).key(format!("tool-pattern-{id}")),
            );
            let results = tool.structured["fileSearchResults"].as_array();
            if let Some(results) = results.filter(|r| !r.is_empty()) {
                let mut list = col().min_w(0.0).gap(4.0);
                for (i, result) in results.iter().enumerate() {
                    let path = result["fileName"]
                        .as_str()
                        .or_else(|| result["path"].as_str())
                        .unwrap_or("");
                    let mut path = relative(path, &root);
                    if let Some(line) = result["line"].as_u64() {
                        path.push_str(&format!(":{line}"));
                        if let Some(column) = result["column"].as_u64() {
                            path.push_str(&format!(":{column}"));
                        }
                    }
                    let mut entry = col()
                        .key(format!("tool-search-result-{id}-{i}"))
                        .min_w(0.0)
                        .child(mono_dim(path, p));
                    if let Some(preview) = result["preview"].as_str() {
                        entry = entry.child(
                            label(preview, SMALL, p.muted)
                                .min_w(0.0)
                                .wrap_anywhere()
                                .selectable(true),
                        );
                    }
                    list = list.child(entry);
                }
                body = body.child(block(format!("tool-search-results-{id}"), list, false));
            } else {
                body = body.child(output(tool, p, &id));
            }
        }
        ToolKind::WebSearch => {
            // KSG's web_search inspector: queries in mono, then medium titles with an
            // external-link glyph and a muted snippet.
            let query = input_text(tool, &["query", "url", "queries", "patterns"]);
            body = body.child(mono(query, p).key(format!("tool-query-{id}")));
            let results = tool.structured["webResults"].as_array();
            if let Some(results) = results.filter(|r| !r.is_empty()) {
                let mut list = col().min_w(0.0).gap(6.0);
                for (i, result) in results.iter().enumerate() {
                    let url = result["url"].as_str().unwrap_or("");
                    let title = result["title"].as_str().unwrap_or(url);
                    let mut head = row().center_y().gap(4.0).min_w(0.0).child(
                        label(title, TextStyle::new(12.0, 500.0), p.ink)
                            .min_w(0.0)
                            .truncate()
                            .key(format!("tool-web-link-{id}-{i}")),
                    );
                    if url.starts_with("https://") || url.starts_with("http://") {
                        // Rune's href dispatches through Tx::open_url, including keyboard activation.
                        head = head
                            .child(icon("external-link", p.ink).size(12.0, 12.0))
                            .href(url)
                            .focusable(true)
                            .cursor(rune::Cursor::Pointer)
                            .hover(p.selected);
                    }
                    let mut entry = col().min_w(0.0).gap(2.0).child(head);
                    if let Some(snippet) = result["snippet"].as_str() {
                        entry = entry.child(
                            label(snippet, SMALL, p.muted)
                                .min_w(0.0)
                                .wrap_anywhere()
                                .selectable(true),
                        );
                    }
                    list = list.child(entry);
                }
                body = body.child(block(format!("tool-web-results-{id}"), list, false));
            } else {
                body = body.child(output(tool, p, &id));
            }
        }
        _ => {
            // KSG's ToolCallBody args: a muted key, then the value inline in fg/85 or as
            // a muted StructuredValue block when it carries JSON or newlines.
            let mut fields = col().min_w(0.0).gap(6.0);
            if let Some(input) = tool.input.as_object().filter(|i| !i.is_empty()) {
                for (key, value) in input {
                    let text = value
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| value.to_string());
                    if value.is_string() && !looks_like_json(&text) && !text.contains('\n') {
                        fields = fields.child(
                            row()
                                .min_w(0.0)
                                .child(mono_muted(format!("{key} "), p))
                                .child(mono(text, p)),
                        );
                    } else {
                        fields = fields.child(
                            col()
                                .min_w(0.0)
                                .gap(2.0)
                                .child(mono_muted(format!("{key} "), p))
                                .child(block(
                                    format!("tool-field-{id}-{key}"),
                                    mono_muted(text, p),
                                    false,
                                )),
                        );
                    }
                }
            } else if let Some(text) = tool.input.as_str().filter(|v| !v.is_empty()) {
                fields = fields.child(mono_muted(text, p));
            } else if !tool.detail.is_empty() {
                fields = fields.child(mono(&tool.detail, p));
            }
            body = body
                .child(block(format!("tool-fields-{id}"), fields, !tool.done))
                .child(output(tool, p, &id));
        }
    }
    if let Some(exit) = tool.exit_code.filter(|exit| *exit != 0) {
        // KSG's ToolCallBody exit line, mono and destructive.
        body = body.child(
            label(format!("exit {exit}"), mono_style(), p.danger)
                .key(format!("tool-exit-{id}"))
                .min_w(0.0)
                .wrap_anywhere()
                .selectable(true),
        );
    }
    if let Some(parent) = &tool.parent_call {
        body = body.child(label(format!("Called by {parent}"), SMALL, p.muted).wrap_anywhere());
    }
    if !tool.images.is_empty() {
        body = body.child(message_images(
            s,
            p,
            &tool.images,
            &format!("tool-image-{id}"),
        ));
    }
    body.child(raw(s, p, index))
}

/// Ported from KSG's HtmlRenderFrame and HtmlRenderTimelineRow placement.
/// The native-specific snapshot/open presentation follows the HTML rendering brief.
pub(super) fn html_card(s: &State, p: Palette, index: usize) -> El {
    let t = s.current();
    let Some(reference) = crate::html_render::find_reference(&t.tools[index].structured)
        .filter(|r| r.thread_id == t.id)
    else {
        return label("HTML render unavailable", SMALL, p.muted);
    };
    let mut body = col().min_w(0.0).gap(8.0).padding(12.0).child(
        row()
            .center_y()
            .gap(8.0)
            .min_w(0.0)
            .child(meta_text("HTML", p.muted))
            .child(
                label(&reference.title, LABEL, p.ink)
                    .truncate()
                    .min_w(0.0)
                    .shrink(),
            )
            .child(spacer())
            .child(icon("open", p.sea)),
    );
    if let Some(snapshot) = reference.snapshot(s.dark)
        && s.data_dir
            .join("attachments")
            .join(&snapshot.blob.id)
            .is_file()
    {
        let handle = s.attachment_cache.file(
            s.data_dir.join("attachments").join(&snapshot.blob.id),
            snapshot.width.max(snapshot.height) as f32,
            1.0,
        );
        if rune::is_headless() {
            let _ = handle.wait();
        }
        // Crop below the 480 pt cap. Do not shrink a tall page to fit its whole height.
        body = body.child(
            col()
                .key(format!("html-snapshot-{}-{index}", t.id))
                .min_w(0.0)
                .max_h(480.0)
                .clip_rounded(0.0)
                .child(
                    el().key(format!("html-snapshot-image-{}-{index}", t.id))
                        .w_full()
                        .aspect_ratio(snapshot.width as f32 / snapshot.height as f32)
                        .pixels_fit(handle.pixels(), rune::SurfaceFit::Fit),
                ),
        );
    } else {
        body = body.child(
            label("Open to view", SMALL, p.muted).key(format!("html-placeholder-{}-{index}", t.id)),
        );
    }
    let key = format!("html-card-{}-{index}", t.id);
    framed(body.fill(p.card).radius(12.0), p)
        .key(key)
        .role("button")
        .label(format!("Open {}", reference.title))
        .focusable(true)
        .cursor(rune::Cursor::Pointer)
        .hover(p.selected)
        .press(p.selected)
        .on_press(move |s: &mut State| {
            s.html_windows.open_async(
                s.data_dir.join("attachments"),
                reference.clone(),
                s.dark,
                s.wake.clone(),
            );
        })
}

fn raw(s: &State, p: Palette, index: usize) -> El {
    let tool = &s.current().tools[index];
    let tool_id = tool.id.clone();
    let id = format!("{}-{index}", s.current().id);
    let mut body = col().min_w(0.0).gap(4.0).child(
        row()
            .key(format!("tool-raw-toggle-{id}"))
            .role("button")
            .label("Raw")
            .expanded(tool.raw_expanded)
            .focusable(true)
            .cursor(rune::Cursor::Pointer)
            .center_y()
            .gap(4.0)
            .px(6.0)
            .ml(-6.0)
            .py(4.0)
            .radius(6.0)
            .hover(p.selected)
            .child(meta_text("Raw", p.muted))
            .child(
                icon(if tool.raw_expanded { "chevron" } else { "open" }, p.faint).size(12.0, 12.0),
            )
            .on_press(move |s: &mut State| {
                let thread = s.current_mut();
                if let Some(tool) = thread.tools.iter_mut().find(|tool| tool.id == tool_id) {
                    tool.raw_expanded = !tool.raw_expanded;
                    let message = tool.anchor.message;
                    thread.timeline_cache.get_mut().touch_message(message);
                }
            }),
    );
    if tool.raw_expanded {
        body = body.child(code_well(p).child(block(
            format!("tool-raw-{id}"),
            padded(mono(
                truncate(
                    &serde_json::to_string_pretty(&tool.structured).unwrap_or_default(),
                    50_000,
                ),
                p,
            )),
            false,
        )));
    }
    body
}

/// Ported from KSG Code's `ProposedPlanCard` (`components/chat/ProposedPlanCard.tsx`)
/// and todo_list/proposed_plan inspector. The informational frame follows the brief's outlined
/// card, while Markdown uses Shina's existing renderer.
pub(super) fn plan_card(s: &State, p: Palette, index: usize) -> El {
    let t = s.current();
    let tool = &t.tools[index];
    let id = format!("{}-{index}", t.id);
    let plan = crate::work::plan_data(&tool.input);
    let plan_markdown = plan["markdown"].as_str();
    let displayed = plan_markdown.map(crate::thread_find::plan_parts);
    let query = (s.thread_find_active() && !s.thread_find.query.text.trim().is_empty())
        .then(|| s.thread_find.query.text.trim());
    let active_occ = query.and_then(|_| {
        s.thread_find
            .active_match
            .as_ref()
            .filter(|matched| matched.entry_id == tool.id)
            .map(|matched| matched.occurrence)
    });
    let title = displayed
        .as_ref()
        .map_or("Plan", |(title, _)| title.as_str());
    let mut title_label = label(title, TextStyle::new(14.0, 600.0), p.ink).wrap_anywhere();
    let title_hits = query
        .map(|query| crate::thread_find::find_plain_occurrences(title, query))
        .unwrap_or_default();
    let title_count = title_hits.len();
    if !title_hits.is_empty() {
        title_label =
            title_label.text_highlights(title_hits.into_iter().enumerate().map(|(i, range)| {
                let active = Some(i) == active_occ;
                let mut highlight = rune::TextHighlight::new(
                    range,
                    if active {
                        p.sea.alpha(0.32)
                    } else {
                        p.brass.alpha(0.32)
                    },
                );
                if active {
                    highlight = highlight.key("thread-find-active");
                }
                highlight
            }));
    }
    let mut header = row()
        .min_w(0.0)
        .center_y()
        .gap(8.0)
        .child(icon(kind_icon(ToolKind::Plan), p.muted))
        .child(title_label.grow(1.0).min_w(0.0));
    if plan_markdown.is_some() {
        let menu_open = s.plan_menu.as_deref() == Some(id.as_str());
        let menu_id = id.clone();
        header = header.child(
            toggle_button(
                &format!("plan-actions-{id}"),
                "Plan actions",
                "more",
                menu_open,
                p,
                move |s| {
                    if s.plan_menu.as_deref() == Some(menu_id.as_str()) {
                        s.plan_menu = None;
                    } else {
                        s.plan_menu = Some(menu_id.clone());
                    }
                },
            )
            .size(28.0, 28.0),
        );
    }
    let mut body = col().min_w(0.0).gap(8.0).child(header);
    if let Some(explanation) = plan["explanation"].as_str().filter(|s| !s.is_empty()) {
        body = body.child(
            label(explanation, SMALL, p.muted)
                .wrap_anywhere()
                .selectable(true),
        );
    }
    let mut steps = col().min_w(0.0).gap(8.0);
    for (i, step) in plan["steps"].as_array().into_iter().flatten().enumerate() {
        let status = step["status"].as_str().unwrap_or("pending");
        let word = match status {
            "running" => "In progress",
            "completed" => "Done",
            _ => "Pending",
        };
        let mut square = el()
            .key(format!("plan-step-status-{id}-{i}"))
            .role("img")
            .label(word)
            .size(10.0, 10.0)
            .border(1.0, p.muted);
        square = match status {
            "completed" => square.fill(p.ink),
            "running" => square.child(el().size(5.0, 10.0).fill(p.muted)),
            _ => square,
        };
        steps = steps.child(
            row()
                .key(format!("plan-step-{id}-{i}"))
                .min_w(0.0)
                .gap(8.0)
                .child(el().size(24.0, 20.0).center().child(square))
                .child(
                    col()
                        .min_w(0.0)
                        .grow(1.0)
                        .gap(2.0)
                        .child(
                            label(step["text"].as_str().unwrap_or(""), SMALL, p.ink)
                                .wrap_anywhere()
                                .selectable(true),
                        )
                        .child(meta_text(word, p.muted)),
                ),
        );
    }
    if plan["steps"]
        .as_array()
        .is_some_and(|steps| !steps.is_empty())
    {
        body = body.child(block(format!("plan-steps-{id}"), steps, false));
    }
    let can_collapse = plan_markdown.is_some_and(crate::plan_export::can_collapse_plan);
    let expanded = s.plan_expanded.contains(&id);
    let is_collapsed = can_collapse && !expanded;
    let find_active = s.thread_find_active();
    let show_preview = is_collapsed && !find_active;
    let preview_markdown = show_preview.then(|| {
        crate::plan_export::build_collapsed_proposed_plan_preview_markdown(
            plan_markdown.expect("collapsible plan has markdown"),
            10,
        )
    });
    let markdown_body = if let Some(preview) = preview_markdown.as_deref() {
        Some((preview, false))
    } else {
        displayed
            .as_ref()
            .map(|(_, body)| body.as_str())
            .filter(|body| !body.is_empty())
            .map(|body| (body, true))
    };
    if let Some((markdown, with_find)) = markdown_body {
        let rendered = markdown_ui::render_with_find(
            markdown,
            p,
            s.dark,
            s.viewport_width >= WIDE_PANE,
            false,
            !tool.done,
            &format!("plan-{id}"),
            &s.unwrapped_code_blocks,
            with_find
                .then(|| {
                    query.map(|query| {
                        (
                            query,
                            active_occ.and_then(|occ| occ.checked_sub(title_count)),
                        )
                    })
                })
                .flatten(),
        );
        let markdown_el = if can_collapse && (expanded || find_active) {
            col()
                .key(format!("plan-markdown-{id}"))
                .min_w(0.0)
                .child(rendered)
        } else {
            block(format!("plan-markdown-{id}"), rendered, !tool.done)
        };
        body = body.child(markdown_el);
    }
    if can_collapse {
        let collapse_id = id.clone();
        body = body.child(
            secondary_button(
                &format!("plan-collapse-{id}"),
                if expanded {
                    "Collapse plan"
                } else {
                    "Expand plan"
                },
                p,
                move |s| s.toggle_plan_expanded(&collapse_id),
            )
            .w_full()
            .justify_center(),
        );
    }
    col()
        .key(format!("inline-tool-{id}"))
        .min_w(0.0)
        .child(outlined(
            col()
                .key(format!("plan-card-{id}"))
                .min_w(0.0)
                .radius(12.0)
                .child(body.padding(12.0)),
            p,
        ))
}

/// Ported from KSG Code's `ProposedPlanCard` actions menu, reusing `thread_menu` and `menu_item`.
pub(super) fn plan_menu(s: &State, p: Palette, cx: &Cx<State>) -> Option<El> {
    let id = s.plan_menu.clone()?;
    let anchor = format!("plan-actions-{id}");
    // Under Rune #1339 an anchored modal whose trigger scrolled out of the
    // virtualized timeline is suppressed visually while still holding focus.
    // Dismiss the state on layout instead of mounting an invisible modal or
    // leaving `plan_menu` stale.
    if cx.layout_with(&anchor, |_| ()).is_none() {
        return Some(
            el().key("plan-menu-dismiss")
                .absolute(0.0, 0.0)
                .size(0.0, 0.0)
                .a11y_hidden()
                .on_layout(|s: &mut State, _| s.plan_menu = None),
        );
    }
    let t = s.current();
    let prefix = format!("{}-", t.id);
    let index = id
        .strip_prefix(&prefix)
        .and_then(|raw| raw.parse::<usize>().ok())?;
    let tool = t.tools.get(index)?;
    let plan = crate::work::plan_data(&tool.input);
    let plan_markdown = plan["markdown"].as_str()?;
    let thread_id = t.id;
    let export_contents = crate::plan_export::normalize_plan_markdown_for_export(plan_markdown);
    let download_filename =
        crate::plan_export::build_proposed_plan_markdown_filename(plan_markdown);
    let copied = s.is_plan_copied(&id);
    let can_save =
        s.can_write_project_file(t) && !s.plan_save_dialog.as_ref().is_some_and(|d| d.saving);

    Some(
        col()
            .key(format!("plan-menu-{id}"))
            .role("menu")
            .focus_group()
            .label("Plan actions")
            .w(212.0)
            .padding(4.0)
            .gap(1.0)
            .radius(12.0)
            .fill(p.card)
            .border(1.0, p.frame)
            .box_shadow(soft_shadow(p))
            .anchored_fit(anchor, rune::Side::Bottom, rune::Align::End, 4.0)
            .modal(true, |s: &mut State| s.plan_menu = None)
            .child(
                menu_item(
                    &format!("plan-copy-{id}"),
                    if copied {
                        "Copied!"
                    } else {
                        "Copy to clipboard"
                    },
                    if copied { "check" } else { "copy" },
                    p,
                )
                .on_press_tx({
                    let copy_id = id.clone();
                    let contents = export_contents.clone();
                    move |s: &mut State, tx| {
                        tx.copy(contents.clone());
                        s.mark_plan_copied(copy_id.clone(), tx);
                        s.plan_menu = None;
                    }
                }),
            )
            .child(
                menu_item(
                    &format!("plan-download-{id}"),
                    "Download as markdown",
                    "file",
                    p,
                )
                .on_press_tx({
                    let contents = export_contents.clone();
                    let filename = download_filename.clone();
                    move |s: &mut State, tx| {
                        s.plan_menu = None;
                        let contents = contents.clone();
                        tx.save_file(
                            rune::FileDialogOptions {
                                title: "Download as markdown".into(),
                                directory: crate::projects::downloads_dir(),
                                file_name: Some(filename.clone()),
                                ..Default::default()
                            }
                            .filter("Markdown", &["md"]),
                            move |s: &mut State, result, _| match result {
                                Ok(Some(path)) => {
                                    if let Err(error) = std::fs::write(&path, contents.as_bytes()) {
                                        s.push_toast(
                                            crate::model::ToastKind::Error,
                                            "Could not download plan",
                                            Some(error.to_string()),
                                            None,
                                            Some(5_000),
                                        );
                                    }
                                }
                                Ok(None) => {}
                                Err(error) => {
                                    s.push_toast(
                                        crate::model::ToastKind::Error,
                                        "Could not download plan",
                                        Some(error.to_string()),
                                        None,
                                        Some(5_000),
                                    );
                                }
                            },
                        );
                    }
                }),
            )
            .child(
                menu_item(&format!("plan-save-{id}"), "Save to workspace", "folder", p)
                    .disabled(!can_save)
                    .on_press({
                        let markdown = plan_markdown.to_owned();
                        move |s: &mut State| {
                            s.plan_menu = None;
                            s.open_plan_save_dialog(thread_id, &markdown);
                        }
                    }),
            ),
    )
}

/// Ported from KSG Code's `SavePlanToWorkspaceForm`, reusing `thread_dialog` structure and styling.
pub(super) fn plan_save_dialog(s: &State, p: Palette) -> El {
    let dialog = s.plan_save_dialog.as_ref().expect("plan save dialog");
    let card = col()
        .key("plan-save-dialog")
        .role("dialog")
        .label("Save plan to workspace")
        .w(380.0)
        .max_w(540.0)
        .padding(16.0)
        .gap(12.0)
        .radius(12.0)
        .fill(p.card)
        .border(1.0, p.frame)
        .box_shadow(soft_shadow(p))
        .on_press(|_: &mut State| {})
        .child(label("Save plan to workspace", DIALOG_TITLE, p.ink))
        .child(
            label(
                format!("Enter a path relative to {}", dialog.cwd),
                SMALL,
                p.muted,
            )
            .wrap(),
        )
        .child(
            text_input(&dialog.path_edit, BODY)
                .key("plan-save-path-input")
                .label("Workspace path")
                .placeholder("docs/plan.md", p.muted)
                .single_line()
                .color(p.ink)
                .w_full()
                .padding(8.0)
                .radius(12.0)
                .fill(if s.dark { p.bg } else { p.panel })
                .field_focus(&dialog.path_edit, p)
                .on_edit(|s: &mut State| {
                    &mut s
                        .plan_save_dialog
                        .as_mut()
                        .expect("plan save dialog")
                        .path_edit
                })
                .on_submit(|s: &mut State, _| s.submit_plan_save()),
        )
        .child(
            row()
                .gap(8.0)
                .child(spacer())
                .child(
                    secondary_button("cancel-plan-save", "Cancel", p, |s| {
                        s.close_plan_save_dialog()
                    })
                    .disabled(dialog.saving),
                )
                .child(
                    button("confirm-plan-save", "Save", p, |s| s.submit_plan_save())
                        .disabled(dialog.saving),
                ),
        );
    el().key("plan-save-dialog-backdrop")
        .instant_changes()
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .center()
        .padding(16.0)
        .fill(Color::BLACK.alpha(0.3))
        .modal(false, |s: &mut State| s.close_plan_save_dialog())
        .child(card)
}
