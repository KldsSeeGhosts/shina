//! Expanded work content ported from KSG Code's V2ItemInspector and ToolCallBody.
use super::*;
use crate::work::ToolKind;
use serde_json::Value;
#[cfg(test)]
#[path = "work_ui_tests.rs"]
mod tests;

pub(super) fn kind_icon(kind: ToolKind) -> &'static str {
    match kind {
        ToolKind::Command => "terminal",
        ToolKind::FileChange => "edit",
        ToolKind::FileSearch | ToolKind::WebSearch => "search",
        ToolKind::Read => "eye",
        ToolKind::List => "folder",
        ToolKind::Subagent => "crew",
        ToolKind::Plan => "list",
        ToolKind::Other => "wrench",
    }
}

fn mono(value: impl Into<String>, p: Palette) -> El {
    label(
        value,
        TextStyle::new(12.0, 400.0)
            .family(rune::Family::SystemMono)
            .lh(18.0),
        p.ink,
    )
    .min_w(0.0)
    .wrap_anywhere()
    .selectable(true)
}

fn block(key: String, content: El, running: bool) -> El {
    col()
        .key(key)
        .min_w(0.0)
        .max_h(200.0)
        .scroll(ScrollSpec {
            stick_bottom: running,
        })
        .child(content)
}

fn padded(content: El) -> El {
    content.px(12.0).py(9.0)
}

fn relative(path: &str, root: &std::path::Path) -> String {
    // Projection paths belong to their host. Do not canonicalize or read their files.
    let root = root.to_string_lossy().replace('\\', "/");
    let path = path.replace('\\', "/");
    path.strip_prefix(&format!("{}/", root.trim_end_matches('/')))
        .unwrap_or(&path)
        .to_owned()
}

fn input_text(tool: &Tool, keys: &[&str]) -> String {
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
        padded(mono(text, p)),
        !tool.done,
    )
}

/// The existing output frame owns padding and caps, not a second inspector or drawer.
pub(super) fn expanded(s: &State, p: Palette, index: usize) -> El {
    let t = s.current();
    let tool = &t.tools[index];
    let id = format!("{}-{index}", t.id);
    let root = s.working_directory();
    let mut body = col()
        .key(format!("tool-output-{id}"))
        .role("group")
        .label(format!("Output for {}", tool_title(tool)))
        .min_w(0.0)
        .mb(4.0)
        .ml(28.0)
        .radius(6.0)
        .fill(p.code_bg)
        .border(1.0, p.line);
    match tool.resolved_kind() {
        ToolKind::Command => {
            body = body
                .child(block(
                    format!("tool-command-{id}"),
                    padded(
                        col()
                            .min_w(0.0)
                            .gap(4.0)
                            .child(pixel_text("sh", p.muted))
                            .child(mono(input_text(tool, &["command", "cmd"]), p)),
                    ),
                    false,
                ))
                .child(rule(p))
                .child(output(tool, p, &id));
        }
        ToolKind::FileChange if !tool.changes.is_empty() => {
            for (file, change) in tool.changes.iter().enumerate() {
                if file > 0 {
                    body = body.child(rule(p));
                }
                let mut heading = col().min_w(0.0).gap(4.0).child(
                    mono(relative(&change.path, &root), p)
                        .key(format!("tool-file-path-{id}-{file}")),
                );
                if let Some(old) = &change.old_path {
                    heading =
                        heading.child(mono(format!("Previously {}", relative(old, &root)), p));
                }
                let mut meta = row().gap(8.0).child(pixel_text(&change.operation, p.muted));
                if !change.diff.is_empty() {
                    meta = meta.child(pixel_text(
                        format!("+{} -{}", change.additions, change.deletions),
                        p.muted,
                    ));
                }
                body = body.child(padded(heading.child(meta)));
                if !change.diff.is_empty() {
                    let mut lines = col().min_w(0.0).py(6.0);
                    for (line, value) in change.diff.lines().enumerate() {
                        let mut content = mono(if value.is_empty() { " " } else { value }, p)
                            .key(format!("tool-diff-line-{id}-{file}-{line}"))
                            .px(12.0);
                        let added = value.starts_with('+') && !value.starts_with("+++");
                        let removed = value.starts_with('-') && !value.starts_with("---");
                        if added || removed {
                            // Palette has no positive/negative pair. Use the requested
                            // monochrome fallback; the source retains its +/- gutter.
                            let ink = if added { p.ink } else { p.muted };
                            content = content.color(ink).border_sides([0.0, 0.0, 0.0, 2.0], ink);
                        }
                        lines = lines.child(content);
                    }
                    body = body.child(rule(p)).child(block(
                        format!("tool-diff-{id}-{file}"),
                        lines,
                        false,
                    ));
                }
            }
        }
        ToolKind::FileSearch => {
            body = body
                .child(block(
                    format!("tool-pattern-{id}"),
                    padded(mono(input_text(tool, &["pattern", "query"]), p)),
                    false,
                ))
                .child(rule(p));
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
                    }
                    let mut entry = col()
                        .key(format!("tool-search-result-{id}-{i}"))
                        .min_w(0.0)
                        .child(mono(path, p));
                    if let Some(preview) = result["preview"].as_str() {
                        entry = entry.child(mono(preview, p));
                    }
                    list = list.child(entry);
                }
                body = body.child(block(
                    format!("tool-search-results-{id}"),
                    padded(list),
                    false,
                ));
            } else {
                body = body.child(output(tool, p, &id));
            }
        }
        ToolKind::WebSearch => {
            let query = input_text(tool, &["query", "url", "queries", "patterns"]);
            body = body
                .child(block(
                    format!("tool-query-{id}"),
                    padded(mono(query, p)),
                    false,
                ))
                .child(rule(p));
            let results = tool.structured["webResults"].as_array();
            if let Some(results) = results.filter(|r| !r.is_empty()) {
                let mut list = col().min_w(0.0).gap(6.0);
                for (i, result) in results.iter().enumerate() {
                    let url = result["url"].as_str().unwrap_or("");
                    let title = result["title"].as_str().unwrap_or(url);
                    let mut title = label(title, SMALL, p.ink)
                        .min_w(0.0)
                        .wrap_anywhere()
                        .key(format!("tool-web-link-{id}-{i}"));
                    if url.starts_with("https://") || url.starts_with("http://") {
                        // Rune's href dispatches through Tx::open_url, including keyboard activation.
                        title = title
                            .href(url)
                            .focusable(true)
                            .cursor(rune::Cursor::Pointer)
                            .hover(p.selected);
                    }
                    let mut entry = col().min_w(0.0).gap(2.0).child(title);
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
                body = body.child(block(format!("tool-web-results-{id}"), padded(list), false));
            } else {
                body = body.child(output(tool, p, &id));
            }
        }
        _ => {
            let mut fields = col().min_w(0.0).gap(4.0);
            if let Some(input) = tool.input.as_object().filter(|i| !i.is_empty()) {
                for (key, value) in input {
                    fields = fields.child(
                        col()
                            .min_w(0.0)
                            .child(label(key, SMALL, p.muted))
                            .child(mono(
                                value
                                    .as_str()
                                    .map(str::to_owned)
                                    .unwrap_or_else(|| value.to_string()),
                                p,
                            )),
                    );
                }
            } else {
                fields = fields.child(mono(&tool.detail, p));
            }
            body = body
                .child(block(format!("tool-fields-{id}"), padded(fields), false))
                .child(rule(p))
                .child(output(tool, p, &id));
        }
    }
    if let Some(exit) = tool.exit_code.filter(|exit| *exit != 0) {
        body = body.child(
            padded(pixel_text(format!("Exit code {exit}"), p.muted)).key(format!("tool-exit-{id}")),
        );
    }
    if let Some(parent) = &tool.parent_call {
        body = body.child(padded(
            label(format!("Called by {parent}"), SMALL, p.muted).wrap_anywhere(),
        ));
    }
    if !tool.images.is_empty() {
        body = body.child(message_images(
            s,
            p,
            &tool.images,
            &format!("tool-image-{id}"),
        ));
    }
    body = body.child(rule(p)).child(
        row()
            .key(format!("tool-raw-toggle-{id}"))
            .role("button")
            .label("Raw")
            .expanded(tool.raw_expanded)
            .focusable(true)
            .cursor(rune::Cursor::Pointer)
            .center_y()
            .gap(4.0)
            .px(12.0)
            .py(8.0)
            .hover(p.selected)
            .child(pixel_text("Raw", p.muted))
            .child(icon(
                if tool.raw_expanded { "chevron" } else { "open" },
                p.faint,
            ))
            .on_press(move |s: &mut State| {
                let tool = &mut s.current_mut().tools[index];
                tool.raw_expanded = !tool.raw_expanded;
            }),
    );
    if tool.raw_expanded {
        body = body.child(block(
            format!("tool-raw-{id}"),
            padded(mono(
                truncate(
                    &serde_json::to_string_pretty(&tool.structured).unwrap_or_default(),
                    50_000,
                ),
                p,
            )),
            false,
        ));
    }
    body
}
