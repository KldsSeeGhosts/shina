//! Port of KSG's TimelineUserMessageRow, UserMessageContextChips and contextChipParts.
//! Static Rune pills keep references inline, selectable and keyboard-accessible.
use super::*;
use crate::composer_context::{
    self as context, ComposerContextData as Data, ComposerContextRecord as Record,
};
use crate::model::Message;
use pulldown_cmark::{Event, Options, Parser, Tag};
use rune_kit::markdown::{Block, Inline, Style};
use std::collections::{HashMap, HashSet};
use std::ops::Range;

pub(crate) struct Preview {
    title: String,
    text: String,
    image: Option<Attachment>,
}

fn resolved(message: &Message) -> (String, Vec<Record>) {
    match &message.context {
        Some(context) => (message.text.clone(), context.records.clone()),
        None => context::upgrade_legacy_context_message(&message.text),
    }
}

pub(super) fn has_context(message: &Message) -> bool {
    !context::collect_composer_context_references(&resolved(message).0).is_empty()
}

pub(super) fn unchipped_attachments(message: &Message) -> Vec<Attachment> {
    let (text, records) = resolved(message);
    let ids: HashSet<_> = context::collect_composer_context_references(&text)
        .into_iter()
        .map(|r| r.context_id)
        .collect();
    let chipped: HashSet<_> = records
        .iter()
        .filter(|r| ids.contains(&r.context_id))
        .filter_map(|r| match &r.data {
            Data::File { attachment_id, .. } => Some(attachment_id.as_str()),
            _ => None,
        })
        .collect();
    // KSG keeps image media above the bubble, but not duplicate file cards.
    message
        .images
        .iter()
        .filter(|a| a.is_image() || !chipped.contains(a.id.as_str()))
        .cloned()
        .collect()
}

fn known<'a>(records: &'a [Record], href: &str) -> Option<&'a Record> {
    let (kind, id) = context::parse_composer_context_href(href)?;
    // KSG's recordsById is a Map, so the last duplicate wins.
    records
        .iter()
        .rev()
        .find(|r| r.context_id == id)
        .filter(|r| r.is_known() && r.kind() == kind)
}

fn available(record: &Record, message: &Message) -> bool {
    match &record.data {
        Data::Image { attachment_id, .. } => message
            .images
            .iter()
            .any(|a| a.id == *attachment_id && a.is_image()),
        Data::File { attachment_id, .. } => message.images.iter().any(|a| {
            a.id == *attachment_id
                && !a.is_image()
                && crate::attachments::path_for(a).is_some_and(|path| path.is_file())
        }),
        _ => true,
    }
}

fn machine_key(s: &State, environment: &str) -> String {
    if environment == s.machine_identity("local") {
        "local".into()
    } else {
        environment.into()
    }
}

fn display_label(s: &State, record: &Record, fallback: &str) -> String {
    match &record.data {
        Data::Thread {
            environment_id,
            thread_id,
            title,
        } => {
            let machine = machine_key(s, environment_id);
            let threads = if machine == s.machines.selected {
                Some(&s.threads)
            } else {
                s.machines.threads.get(&machine)
            };
            threads
                .and_then(|threads| threads.iter().find(|t| t.id.to_string() == *thread_id))
                .map(|t| t.title.trim())
                .filter(|t| !t.is_empty())
                .unwrap_or(title)
                .to_owned()
        }
        Data::Image {
            name, size_bytes, ..
        }
        | Data::File {
            name, size_bytes, ..
        } => format!("{} {}", middle_name(name), size_label(*size_bytes)),
        Data::ReviewComment {
            pull_request: Some(pr),
            ..
        } => format!("#{}", pr.number),
        _ => {
            if record.label.is_empty() {
                fallback.into()
            } else {
                record.label.clone()
            }
        }
    }
}

fn size_label(bytes: u64) -> String {
    let kb = bytes.div_ceil(1024).max(1);
    if kb >= 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{kb} KB")
    }
}

fn middle_name(name: &str) -> String {
    // KSG's composerInlineChip.middleTruncateAttachmentName.
    let chars: Vec<_> = name.chars().collect();
    if chars.len() <= 36 {
        name.into()
    } else {
        format!(
            "{}…{}",
            chars[..21].iter().collect::<String>(),
            chars[chars.len() - 14..].iter().collect::<String>()
        )
    }
}

#[derive(Clone)]
struct CopyLink {
    range: Range<usize>,
    markdown: String,
    context_id: String,
}
#[derive(Clone)]
struct CopyText {
    text: String,
    links: Vec<CopyLink>,
}

struct Document<'a> {
    state: &'a State,
    message: &'a Message,
    records: Vec<Record>,
    blocks: Vec<Block>,
    code: Vec<(String, String)>,
    copies: HashMap<String, CopyText>,
    counter: usize,
    wide: bool,
    palette: Palette,
    max_width: f32,
}

impl<'a> Document<'a> {
    fn new(state: &'a State, message: &'a Message, wide: bool, palette: Palette) -> Self {
        let (wide, max_width) = state
            .transcript_layout
            .get(&message.id)
            .copied()
            .unwrap_or((wide, 500.0));
        let (mut text, records) = resolved(message);
        // Markdown images normally become media blocks. Only actual context-image syntax
        // becomes an inline reference; code fences and inline code remain literal.
        let mut image_starts: Vec<_> = Parser::new_ext(&text, Options::all())
            .into_offset_iter()
            .filter_map(|(event, range)| match event {
                Event::Start(Tag::Image { dest_url, .. })
                    if context::parse_composer_context_href(&dest_url).is_some() =>
                {
                    Some(range.start)
                }
                _ => None,
            })
            .collect();
        image_starts.sort_unstable();
        for at in image_starts.into_iter().rev() {
            text.remove(at);
        }
        let text = markdown_ui::user_source(&text).into_owned();
        let mut code = Vec::new();
        let mut fence = None;
        for (event, range) in Parser::new_ext(&text, Options::all()).into_offset_iter() {
            match event {
                Event::Start(Tag::CodeBlock(_)) => fence = Some((range.start, String::new())),
                Event::Text(body) => {
                    if let Some((_, text)) = &mut fence {
                        text.push_str(&body);
                    }
                }
                Event::End(pulldown_cmark::TagEnd::CodeBlock) => {
                    if let Some((start, body)) = fence.take() {
                        code.push((body, text[start..range.end].into()));
                    }
                }
                _ => {}
            }
        }
        Self {
            state,
            message,
            records,
            blocks: rune_kit::markdown::parse(&text),
            code,
            copies: HashMap::new(),
            counter: 0,
            wide,
            palette,
            max_width,
        }
    }

    fn blocks(&mut self, blocks: &[Block]) -> El {
        let mut out = col().w_full().min_w(0.0).gap(12.0);
        for block in blocks {
            out = out.child(self.block(block));
        }
        out
    }

    fn block(&mut self, block: &Block) -> El {
        match block {
            Block::Para(inline) => self.inline(inline, body_style(self.wide)),
            Block::Heading(level, inline) => self.inline(
                inline,
                TextStyle::new(
                    match level {
                        1 => 24.0,
                        2 => 20.0,
                        _ => 16.0,
                    },
                    600.0,
                ),
            ),
            Block::List { start, items } => {
                let mut list = col().w_full().min_w(0.0).gap(8.0);
                for (i, item) in items.iter().enumerate() {
                    let marker = start.map_or_else(|| "•".into(), |n| format!("{}.", n + i as u64));
                    list = list.child(
                        row()
                            .w_full()
                            .min_w(0.0)
                            .gap(8.0)
                            .child(label(marker, body_style(self.wide), self.palette.ink))
                            .child(self.blocks(item).grow(1.0).min_w(0.0)),
                    );
                }
                list
            }
            Block::Quote(blocks) => row()
                .w_full()
                .min_w(0.0)
                .gap(12.0)
                .child(el().w(2.0).h_full().fill(self.palette.muted))
                .child(self.blocks(blocks).grow(1.0).min_w(0.0)),
            Block::Table(table) => {
                let mut table_el = col().w_full().min_w(0.0).gap(4.0);
                for (n, cells) in std::iter::once(&table.head).chain(&table.rows).enumerate() {
                    let mut row_el = row().w_full().min_w(0.0).gap(12.0);
                    for cell in cells {
                        row_el = row_el.child(
                            self.inline(
                                cell,
                                TextStyle::new(14.0, if n == 0 { 600.0 } else { 400.0 }),
                            )
                            .w(0.0)
                            .grow(1.0)
                            .min_w(0.0),
                        );
                    }
                    table_el = table_el.child(row_el);
                }
                table_el
            }
            Block::Rule => rule(self.palette),
            Block::Code(body) | Block::Diff(body) | Block::Mermaid(body) => {
                let source = self
                    .code
                    .iter()
                    .find(|(text, _)| text == body)
                    .map(|(_, src)| src.clone())
                    .unwrap_or_else(|| format!("```\n{body}\n```"));
                let identity = format!(
                    "context-code-{}-{}-{}",
                    self.state.current().id,
                    self.message.id,
                    self.counter
                );
                self.counter += 1;
                markdown_ui::render_with_find(
                    &source,
                    self.palette,
                    self.state.dark,
                    self.wide,
                    true,
                    false,
                    &identity,
                    None,
                )
            }
            Block::Image { alt, .. } => {
                label(alt, body_style(self.wide), self.palette.muted).wrap_anywhere()
            }
        }
    }

    fn inline(&mut self, inline: &Inline, style: TextStyle) -> El {
        let key = format!(
            "context-text-{}-{}-{}",
            self.state.current().id,
            self.message.id,
            self.counter
        );
        self.counter += 1;
        let p = self.palette;
        let mut text = inline.text.clone();
        let mut links = inline.links.clone();
        let original_labels: Vec<_> = links
            .iter()
            .map(|(a, b, _)| inline.text[*a..*b].to_string())
            .collect();
        let mut spans = inline.spans.clone();
        // Use live thread titles, and KSG's file-name and size labels. Adjust ranges so
        // surrounding Markdown formatting continues to address the rendered bytes.
        for i in (0..links.len()).rev() {
            let (a, b, href) = &links[i];
            if context::parse_composer_context_href(href).is_some() {
                let label = known(&self.records, href)
                    .filter(|r| available(r, self.message))
                    .map_or_else(
                        || text[*a..*b].to_string(),
                        |record| display_label(self.state, record, &text[*a..*b]),
                    );
                let label = fit_label(&label, style, self.max_width);
                let (a, b) = (*a, *b);
                let delta = label.len() as isize - (b - a) as isize;
                text.replace_range(a..b, &label);
                let shift = |v: usize| {
                    if v >= b {
                        v.saturating_add_signed(delta)
                    } else {
                        v
                    }
                };
                for (x, y, _) in &mut links {
                    *x = shift(*x);
                    *y = shift(*y);
                }
                for (x, y, _) in &mut spans {
                    *x = shift(*x);
                    *y = shift(*y);
                }
            }
        }
        let painted = spans
            .iter()
            .map(|(a, b, st)| {
                let span = rune::Span::new(*a, *b);
                match st {
                    Style::Strong => span.bold(),
                    Style::Em => span.italic(),
                    Style::Code => span.mono().bg(p.code_bg, 4.0),
                    Style::Link => span.color(p.ink),
                    _ => span,
                }
            })
            .collect::<Vec<_>>();
        let mut el = rune::rich(&text, painted)
            .key(&key)
            .font(style)
            .color(p.ink)
            .wrap_anywhere()
            .w_full()
            .min_w(0.0)
            .selectable(true);
        for &(a, b, style) in &spans {
            if style == Style::Strike {
                el = el.strike(a, b, None);
            }
            if style == Style::CodePad {
                el = el.uncopied(a..b);
            }
        }
        if self.state.thread_find_active() {
            let query = self.state.thread_find.query.text.trim();
            if !query.is_empty() {
                el = el.text_highlights(
                    crate::thread_find::find_plain_occurrences(&text, query)
                        .into_iter()
                        .filter(|range| {
                            !links.iter().any(|(a, b, href)| {
                                context::parse_composer_context_href(href).is_some()
                                    && range.start < *b
                                    && range.end > *a
                            })
                        })
                        .map(|range| rune::TextHighlight::new(range, p.brass.alpha(0.32))),
                );
            }
        }
        let mut copied = Vec::new();
        for (index, (a, b, href)) in links.into_iter().enumerate() {
            let Some((kind, id)) = context::parse_composer_context_href(&href) else {
                el = el.link(rune::Link::new(a..b, href));
                continue;
            };
            let record = known(&self.records, &href).filter(|r| available(r, self.message));
            let status = if record.is_some() {
                composer_menu::ChipStatus::Resolved
            } else {
                composer_menu::ChipStatus::Unresolved
            };
            let (bg, fg, icon) = composer_menu::pill_appearance(
                status,
                &href,
                &self.records,
                &self.message.images,
                &self.state.current().machine,
                p,
            );
            el = el.pill(a..b, rune::PillStyle::default().bg(bg).fg(fg).icon(icon));
            copied.push(CopyLink {
                range: a..b,
                markdown: context::format_composer_context_reference(
                    &kind,
                    &id,
                    &original_labels[index],
                ),
                context_id: id,
            });
            if record.is_some() {
                el = el.link(
                    rune::Link::new(a..b, &href)
                        .role("button")
                        .label(format!("{kind}, {}", &text[a..b]))
                        .help(match kind.as_str() {
                            "thread" => "Open thread",
                            "file" | "mention" | "image" => "Preview file",
                            _ => "Show details",
                        })
                        .hover(rune::LinkPaint::new().underline(fg)),
                );
            } else {
                el = el.help("This context is no longer available.");
            }
        }
        self.copies.insert(
            key,
            CopyText {
                text,
                links: copied,
            },
        );
        let message_id = self.message.id.clone();
        el.on_link(move |s: &mut State, href, tx| activate(s, &message_id, href, tx))
            .on_secondary_press(|s: &mut State, at, tx| {
                let plain = tx
                    .text_selection()
                    .map(|selection| rune::ClipboardContent::new(selection.text()));
                let content = tx
                    .text_selection()
                    .map(|selection| selection_payload(s, selection))
                    .transpose()
                    .unwrap_or_else(|error| {
                        s.current_mut().attachment_error = Some(error);
                        None
                    })
                    .flatten()
                    .or(plain);
                let menu = rune::PopupMenu::new()
                    .item(rune::PopupItem::new("copy", "Copy").enabled(content.is_some()));
                tx.popup_menu(
                    menu,
                    rune::PopupAnchor::Point(at.x, at.y),
                    move |_: &mut State, picked, tx| {
                        if picked == Some("copy")
                            && let Some(content) = content
                        {
                            tx.copy_rich(content);
                        }
                    },
                );
            })
    }
}

/// Ported from KSG's TimelineUserMessageRow.renderContextReference.
pub(super) fn render(s: &State, p: Palette, wide: bool, message: &Message, max_width: f32) -> El {
    let mut doc = Document::new(s, message, wide, p);
    doc.wide = wide;
    doc.max_width = max_width;
    let message_id = message.id.clone();
    doc.blocks(&doc.blocks.clone())
        .selection_group(true)
        .on_layout_tx(move |s: &mut State, _, tx| {
            if s.transcript_layout.get(&message_id) != Some(&(wide, max_width)) {
                s.transcript_layout
                    .insert(message_id.clone(), (wide, max_width));
            } else {
                tx.live_only();
            }
        })
}

fn fit_label(label: &str, style: TextStyle, max_width: f32) -> String {
    thread_local! {
        static TEXT: std::cell::RefCell<rune::text::TextSystem> = std::cell::RefCell::new(
            rune::text::TextSystem::new(FONTS, (SANS_FAMILY, "Cascadia Mono, Consolas")));
    }
    // Rune's pill reserves its icon, padding and margins in the text layout.
    let limit = (max_width - style.size * 2.2).max(style.size);
    TEXT.with(|text| {
        let mut text = text.borrow_mut();
        if text.measure(label, style, 1.0).0 <= limit {
            return label.to_owned();
        }
        let mut label = label.to_owned();
        while !label.is_empty() {
            label.pop();
            let fitted = format!("{label}…");
            if text.measure(&fitted, style, 1.0).0 <= limit {
                return fitted;
            }
        }
        "…".into()
    })
}

pub(super) fn activate(s: &mut State, message_id: &str, href: &str, tx: &mut Tx) {
    if context::parse_composer_context_href(href).is_none() {
        let _ = tx.open_url(href);
        return;
    }
    let Some(message) = s.current().messages.iter().find(|m| m.id == message_id) else {
        return;
    };
    let (_, records) = resolved(message);
    let Some(record) = known(&records, href).filter(|r| available(r, message)) else {
        return;
    };
    match &record.data {
        Data::Thread {
            environment_id,
            thread_id,
            ..
        } => {
            if let Ok(id) = thread_id.parse::<usize>() {
                s.open_machine_thread(&machine_key(s, environment_id), id);
            }
        }
        Data::Image { attachment_id, .. } | Data::File { attachment_id, .. } => {
            if let Some(attachment) = message.images.iter().find(|a| a.id == *attachment_id) {
                s.transcript_preview = Some(file_preview(attachment));
            }
        }
        Data::Mention { path } => {
            if s.machines.selected != "local" {
                s.transcript_preview = Some(Preview {
                    title: record.label.clone(),
                    text: "This file is not available on this device.".into(),
                    image: None,
                });
                return;
            }
            let path = std::path::PathBuf::from(path);
            let path = if path.is_absolute() {
                path
            } else {
                s.thread_directory(s.active).unwrap_or_default().join(path)
            };
            if let Ok(attachment) = crate::attachments::attach_file(&path) {
                s.transcript_preview = Some(file_preview(&attachment));
            }
        }
        Data::ReviewComment {
            pull_request: Some(pr),
            ..
        } => {
            let _ = tx.open_url(&pr.url);
        }
        Data::Environment { environment_id } => {
            s.select_machine(&machine_key(s, environment_id));
        }
        Data::Response { citation, .. } => {
            if let Ok(id) = citation.thread_id.parse::<usize>() {
                s.open_machine_thread(&machine_key(s, &citation.environment_id), id);
                tx.scroll_into_view(format!("row-{id}-{}", citation.message_id));
            }
        }
        _ => {
            s.transcript_preview = Some(Preview {
                title: record.label.clone(),
                text: details(record),
                image: None,
            });
        }
    }
}

fn file_preview(attachment: &Attachment) -> Preview {
    use std::io::Read;
    let text = if attachment.is_image() {
        String::new()
    } else {
        crate::attachments::path_for(attachment)
            .ok_or("This file is no longer available.".to_string())
            .and_then(|path| {
                let mut bytes = Vec::new();
                std::fs::File::open(path)
                    .map_err(|e| e.to_string())?
                    .take(1024 * 1024)
                    .read_to_end(&mut bytes)
                    .map_err(|e| e.to_string())?;
                if bytes.contains(&0) {
                    return Err(
                        "This file contains binary data and cannot be shown as text.".into(),
                    );
                }
                String::from_utf8(bytes).map_err(|_| {
                    "This file is not UTF-8 text. Open it in another app to view its contents."
                        .into()
                })
            })
            .unwrap_or_else(|error| error)
    };
    Preview {
        title: attachment.name.clone(),
        text,
        image: attachment.is_image().then(|| attachment.clone()),
    }
}

fn details(record: &Record) -> String {
    match &record.data {
        Data::Terminal {
            terminal_label,
            line_start,
            line_end,
            text,
            ..
        } => format!("{terminal_label}, lines {line_start}-{line_end}\n\n{text}"),
        Data::ReviewComment {
            file_path,
            range_label,
            text,
            diff,
            ..
        } => format!("{file_path}, {range_label}\n\n{text}\n\n{diff}"),
        Data::Element(element) => format!(
            "{}\n{}\n\n{}\n\n{}",
            element.page_url, element.tag_name, element.html_preview, element.styles
        ),
        Data::PreviewAnnotation {
            page_url,
            comment,
            target_summary,
            ..
        } => format!("{page_url}\n\n{comment}\n\n{target_summary}"),
        Data::Response { citation, .. } => citation.text.clone(),
        Data::Skill { name } => name.clone(),
        Data::Environment { environment_id } => environment_id.clone(),
        _ => record.label.clone(),
    }
}

pub(super) fn preview(s: &State, p: Palette) -> El {
    let preview = s.transcript_preview.as_ref().unwrap();
    let mut content = col()
        .key("transcript-context-preview")
        .role("dialog")
        .label(&preview.title)
        .w(520.0)
        .max_w(s.viewport_width - 32.0)
        .max_h(600.0)
        .padding(16.0)
        .gap(12.0)
        .radius(6.0)
        .fill(p.card)
        .border(2.0, p.frame)
        .box_shadow(hard_shadow(p))
        .on_press(|_: &mut State| {})
        .child(
            row()
                .w_full()
                .gap(8.0)
                .child(
                    label(&preview.title, DIALOG_TITLE, p.ink)
                        .truncate()
                        .grow(1.0),
                )
                .child(icon_button(
                    "close-transcript-context-preview",
                    "Close preview",
                    "close",
                    p,
                    |s| s.transcript_preview = None,
                )),
        );
    content = content.child(if let Some(image) = &preview.image {
        image_preview(
            s,
            p,
            image,
            440.0,
            320.0,
            "transcript-context-preview-image".into(),
        )
    } else {
        label(
            &preview.text,
            TextStyle::new(13.0, 400.0).family(rune::Family::SystemMono),
            p.ink,
        )
        .key("transcript-context-preview-text")
        .wrap_anywhere()
        .selectable(true)
        .max_h(480.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
    });
    el().key("transcript-context-preview-overlay")
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .fill(Color::BLACK.alpha(0.35))
        .center()
        .on_press(|s: &mut State| s.transcript_preview = None)
        .modal(true, |s: &mut State| s.transcript_preview = None)
        .child(content)
}

/// KSG's data-markdown-copy selects records by actual selected chip markup, not by
/// matching lookalike text in a code block. Rune transports the fragment in HTML.
pub(crate) fn copy_selection(s: &mut State, event: &rune::KeyEvent, tx: &mut Tx) -> bool {
    if !event.is_command('c') || event.mods.alt || event.mods.shift {
        return false;
    }
    let Some(selection) = tx.text_selection() else {
        return false;
    };
    match selection_payload(s, selection) {
        Ok(Some(content)) => {
            tx.copy_rich(content);
            true
        }
        Ok(None) => false,
        Err(error) => {
            s.current_mut().attachment_error = Some(error);
            true
        }
    }
}

fn selection_payload(
    s: &State,
    selection: &rune::TextSelection,
) -> Result<Option<rune::ClipboardContent>, String> {
    let mut selected = Vec::new();
    let mut records = Vec::new();
    let mut message_id = None;
    for message in &s.current().messages {
        if message.role != "You" || !has_context(message) {
            continue;
        }
        let mut doc = Document::new(s, message, true, palette(s.dark));
        let _ = doc.blocks(&doc.blocks.clone());
        for text in selection.texts().iter().filter(|t| t.is_selected()) {
            let Some(copy) = text.key().and_then(|key| doc.copies.get(key)) else {
                continue;
            };
            let range = text.range();
            let mut body = String::new();
            let mut cursor = range.start;
            for link in &copy.links {
                if link.range.start >= range.end || link.range.end <= range.start {
                    continue;
                }
                body.push_str(&copy.text[cursor..link.range.start.max(cursor)]);
                body.push_str(&link.markdown);
                cursor = link.range.end.min(range.end);
                records.extend(
                    doc.records
                        .iter()
                        .filter(|r| r.context_id == link.context_id)
                        .cloned(),
                );
                message_id = Some(message.id.clone());
            }
            body.push_str(&copy.text[cursor..range.end]);
            selected.push((text.path().to_string(), body));
        }
    }
    if records.is_empty() {
        return Ok(None);
    }
    let bodies: HashMap<_, _> = selected.into_iter().collect();
    let markdown = selection
        .texts()
        .iter()
        .filter(|t| t.is_selected())
        .map(|t| {
            bodies
                .get(t.path())
                .map_or_else(|| t.text().to_string(), Clone::clone)
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut seen = HashSet::new();
    records.retain(|r| seen.insert(r.context_id.clone()));
    let wanted: HashSet<_> = records
        .iter()
        .filter_map(|r| match &r.data {
            Data::File { attachment_id, .. } | Data::Image { attachment_id, .. } => {
                Some(attachment_id.as_str())
            }
            _ => None,
        })
        .collect();
    let mut files = s
        .current()
        .messages
        .iter()
        .flat_map(|m| m.images.iter())
        .filter(|a| wanted.contains(a.id.as_str()))
        .map(|a| (a.id.clone(), a.clone()))
        .collect();
    crate::attachments::stage_clipboard_files(&mut records, &mut files)?;
    let fragment = context::ComposerContextClipboardFragment {
        version: 1,
        source: context::ClipboardSource {
            environment_id: s.machine_identity(&s.machines.selected),
            thread_id: Some(s.current().id.to_string()),
            message_id,
        },
        records,
    };
    let Some(encoded) = context::encode_composer_context_fragment(&fragment) else {
        return Ok(None);
    };
    context::remember_copied_composer_attachments(&fragment.source.environment_id, files);
    let html = context::encode_composer_context_clipboard_html(&markdown, &encoded, None);
    Ok(Some(rune::ClipboardContent::new(markdown).with_html(html)))
}
