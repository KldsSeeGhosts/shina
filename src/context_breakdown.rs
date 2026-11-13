//! Context window breakdown types and estimation math, ported from KSG Code's
//! `packages/shared/src/contextBreakdown.ts`, `packages/contracts/src/providerRuntime.ts`,
//! `apps/server/src/orchestration-v2/Adapters/ClaudeAdapterV2.ts`, and
//! `apps/web/src/lib/contextWindow.ts`.

use crate::model::{Message, Thought, Tool};
use crate::work::ToolKind;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Code and tool output tokenize denser than prose. One ratio keeps the
/// estimate simple; the reported total absorbs the remaining error.
const CHARS_PER_TOKEN: f64 = 3.5;
const CONTEXT_SYSTEM_LABEL: &str = "System";
const VISIBLE_BREAKDOWN_ROWS: usize = 5;
const MAX_BREAKDOWN_ROWS: usize = 32;
const MAX_LABEL_CHARS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BreakdownRowKind {
    Used,
    Estimated,
    Inferred,
    Buffer,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextBreakdownRow {
    pub label: String,
    pub tokens: u64,
    pub kind: BreakdownRowKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextLegendKind {
    Used,
    Estimated,
    Inferred,
    Other,
    Buffer,
    Free,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextLegendItem {
    pub label: String,
    pub tokens: u64,
    pub kind: ContextLegendKind,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ContextCharTotals {
    pub messages: u64,
    pub tool_calls: u64,
    pub tool_output: u64,
    pub reasoning: u64,
    pub edits: u64,
    pub search: u64,
    pub subagents: u64,
}

/// Rows for the context card from Claude CLI's own `/context` categories.
/// Free space and deferred tool schemas are left out: the first follows from the
/// window and the second sits outside it.
pub fn claude_context_breakdown(usage: &Value) -> Option<Vec<ContextBreakdownRow>> {
    let categories = usage["categories"].as_array()?;
    let mut rows = Vec::new();
    for category in categories {
        let kind = match category["kind"].as_str() {
            Some("used") => BreakdownRowKind::Used,
            Some("buffer") => BreakdownRowKind::Buffer,
            _ => continue,
        };
        let Some(raw_name) = category["name"].as_str() else {
            continue;
        };
        let label: String = raw_name.trim().chars().take(MAX_LABEL_CHARS).collect();
        if label.is_empty() {
            continue;
        }
        let Some(tokens) = rounded_positive_tokens(&category["tokens"]) else {
            continue;
        };
        rows.push(ContextBreakdownRow {
            label,
            tokens,
            kind,
        });
        if rows.len() >= MAX_BREAKDOWN_ROWS {
            break;
        }
    }
    if rows.is_empty() { None } else { Some(rows) }
}

fn rounded_positive_tokens(value: &Value) -> Option<u64> {
    if let Some(n) = value.as_u64() {
        return (n > 0).then_some(n);
    }
    let n = value.as_f64()?;
    if !n.is_finite() || n <= 0.0 {
        return None;
    }
    let rounded = n.round();
    if rounded <= 0.0 || rounded > u64::MAX as f64 {
        return None;
    }
    Some(rounded as u64)
}

fn value_chars(value: &Value) -> u64 {
    match value {
        Value::Null => 0,
        Value::String(s) => s.chars().count() as u64,
        Value::Array(arr) if arr.is_empty() => 0,
        other => serde_json::to_string(other)
            .map(|s| s.chars().count() as u64)
            .unwrap_or(0),
    }
}

fn input_or_detail_chars(tool: &Tool) -> u64 {
    if !tool.input.is_null() {
        value_chars(&tool.input)
    } else {
        tool.detail.chars().count() as u64
    }
}

/// Running character totals for one thread's active (post-compaction) items.
pub fn thread_char_totals(
    messages: &[Message],
    tools: &[Tool],
    thoughts: &[Thought],
    compactions: &[Value],
) -> ContextCharTotals {
    let mut totals = ContextCharTotals::default();
    let last_compaction = compactions
        .iter()
        .rfind(|c| c["running"] != true && c["aborted"] != true && c["errorMessage"].is_null());

    let (start_msg, start_offset) = if let Some(comp) = last_compaction {
        let summary = comp["result"]["summary"]
            .as_str()
            .or_else(|| comp["summary"].as_str())
            .unwrap_or("");
        totals.messages += summary.chars().count() as u64;
        let msg = comp["anchor"]["message"]
            .as_u64()
            .or_else(|| comp["historyIndex"].as_u64())
            .map(|n| n as usize)
            .unwrap_or(0);
        let offset = comp["anchor"]["offset"].as_u64().unwrap_or(0) as usize;
        (msg, offset)
    } else {
        (0, 0)
    };

    for message in messages.iter().skip(start_msg) {
        totals.messages += message.display_text().chars().count() as u64;
    }

    let before_compaction = |anchor_msg: usize, anchor_offset: usize| {
        anchor_msg < start_msg || (anchor_msg == start_msg && anchor_offset < start_offset)
    };

    for thought in thoughts {
        if before_compaction(thought.anchor.message, thought.anchor.offset) {
            continue;
        }
        totals.reasoning += thought.text.chars().count() as u64;
    }

    for tool in tools {
        if before_compaction(tool.anchor.message, tool.anchor.offset) {
            continue;
        }
        let output_chars = tool.output.chars().count() as u64;
        match tool.resolved_kind() {
            ToolKind::Command => {
                totals.tool_calls += input_or_detail_chars(tool);
                totals.tool_output += output_chars;
            }
            ToolKind::FileChange => {
                if !tool.changes.is_empty() {
                    for change in &tool.changes {
                        totals.edits += change.path.chars().count() as u64;
                        totals.edits += if change.diff.is_empty() {
                            value_chars(&tool.input)
                        } else {
                            change.diff.chars().count() as u64
                        };
                    }
                } else {
                    totals.edits += tool.detail.chars().count() as u64 + value_chars(&tool.input);
                }
            }
            ToolKind::FileSearch | ToolKind::WebSearch => {
                totals.search += input_or_detail_chars(tool) + output_chars;
            }
            ToolKind::Subagent => {
                totals.subagents += input_or_detail_chars(tool)
                    + output_chars
                    + tool.subagents.iter().map(value_chars).sum::<u64>();
            }
            ToolKind::Plan => {
                totals.messages += input_or_detail_chars(tool);
            }
            ToolKind::Read | ToolKind::List | ToolKind::Html | ToolKind::Other => {
                totals.tool_calls += input_or_detail_chars(tool);
                totals.tool_output += output_chars;
            }
        }
    }

    totals
}

/// Rows for a thread's reported `used_tokens`, largest first. When the sized
/// items already exceed the reported total, they are scaled down to fit it and
/// there is no remainder row. `None` when nothing was sized.
pub fn estimate_context_breakdown(
    totals: &ContextCharTotals,
    used_tokens: u64,
) -> Option<Vec<ContextBreakdownRow>> {
    if used_tokens == 0 {
        return None;
    }
    let categories = [
        ("Messages", totals.messages),
        ("Tool calls", totals.tool_calls),
        ("Tool output", totals.tool_output),
        ("Reasoning", totals.reasoning),
        ("Edits", totals.edits),
        ("Search", totals.search),
        ("Subagents", totals.subagents),
    ];
    let sized: Vec<(&str, f64)> = categories
        .into_iter()
        .map(|(label, chars)| (label, chars as f64 / CHARS_PER_TOKEN))
        .filter(|(_, tokens)| *tokens >= 1.0)
        .collect();
    let sized_total: f64 = sized.iter().map(|(_, tokens)| *tokens).sum();
    if sized_total == 0.0 {
        return None;
    }

    let used_f = used_tokens as f64;
    let scale = if sized_total > used_f {
        used_f / sized_total
    } else {
        1.0
    };
    let mut rows: Vec<ContextBreakdownRow> = sized
        .into_iter()
        .filter_map(|(label, tokens)| {
            let rounded = (tokens * scale).round();
            if rounded <= 0.0 {
                return None;
            }
            Some(ContextBreakdownRow {
                label: label.to_owned(),
                tokens: rounded as u64,
                kind: BreakdownRowKind::Estimated,
            })
        })
        .collect();

    let estimated_sum: u64 = rows.iter().map(|row| row.tokens).sum();
    if used_tokens > estimated_sum {
        rows.push(ContextBreakdownRow {
            label: CONTEXT_SYSTEM_LABEL.to_owned(),
            tokens: used_tokens - estimated_sum,
            kind: BreakdownRowKind::Inferred,
        });
    }

    if rows.is_empty() {
        return None;
    }
    rows.sort_by(|a, b| b.tokens.cmp(&a.tokens));
    rows.truncate(MAX_BREAKDOWN_ROWS);
    Some(rows)
}

/// The card's segments, in bar order: the largest rows, the rest folded into
/// "Other", the compaction buffer, then free space. `None` when the provider
/// gave no breakdown, so the card keeps its plain bar.
pub fn build_context_legend(
    breakdown: Option<&[ContextBreakdownRow]>,
    max_tokens: Option<u64>,
    used_tokens: u64,
) -> Option<Vec<ContextLegendItem>> {
    let rows = breakdown?;
    if rows.is_empty() {
        return None;
    }

    let mut content: Vec<&ContextBreakdownRow> = rows
        .iter()
        .filter(|row| row.kind != BreakdownRowKind::Buffer && row.tokens > 0)
        .collect();
    content.sort_by(|a, b| b.tokens.cmp(&a.tokens));
    let buffer: u64 = rows
        .iter()
        .filter(|row| row.kind == BreakdownRowKind::Buffer)
        .map(|row| row.tokens)
        .sum();
    if content.is_empty() {
        return None;
    }

    // One slot goes to "Other", so a tail of one row is shown as itself.
    let shown_count = if content.len() > VISIBLE_BREAKDOWN_ROWS + 1 {
        VISIBLE_BREAKDOWN_ROWS
    } else {
        content.len()
    };
    let (shown, folded) = content.split_at(shown_count);

    let mut items: Vec<ContextLegendItem> = shown
        .iter()
        .map(|row| ContextLegendItem {
            label: row.label.clone(),
            tokens: row.tokens,
            kind: match row.kind {
                BreakdownRowKind::Used => ContextLegendKind::Used,
                BreakdownRowKind::Estimated => ContextLegendKind::Estimated,
                BreakdownRowKind::Inferred => ContextLegendKind::Inferred,
                BreakdownRowKind::Buffer => ContextLegendKind::Used,
            },
        })
        .collect();

    if !folded.is_empty() {
        items.push(ContextLegendItem {
            label: "Other".to_owned(),
            tokens: folded.iter().map(|row| row.tokens).sum(),
            kind: ContextLegendKind::Other,
        });
    }

    let content_total: u64 = content.iter().map(|row| row.tokens).sum();
    if used_tokens > content_total {
        let unaccounted = used_tokens - content_total;
        if (unaccounted as f64) > (used_tokens as f64) * 0.01 {
            items.push(ContextLegendItem {
                label: "Uncategorized".to_owned(),
                tokens: unaccounted,
                kind: ContextLegendKind::Inferred,
            });
        }
    }

    if buffer > 0 {
        items.push(ContextLegendItem {
            label: "Buffer".to_owned(),
            tokens: buffer,
            kind: ContextLegendKind::Buffer,
        });
    }

    if let Some(max) = max_tokens.filter(|m| *m > 0) {
        let accounted: u64 = items.iter().map(|item| item.tokens).sum();
        if max > accounted {
            items.push(ContextLegendItem {
                label: "Free space".to_owned(),
                tokens: max - accounted,
                kind: ContextLegendKind::Free,
            });
        }
    }

    Some(items)
}

pub fn is_approximate(
    breakdown: Option<&[ContextBreakdownRow]>,
    legend: Option<&[ContextLegendItem]>,
) -> bool {
    breakdown.is_some_and(|rows| {
        rows.iter().any(|row| {
            matches!(
                row.kind,
                BreakdownRowKind::Estimated | BreakdownRowKind::Inferred
            )
        })
    }) || legend.is_some_and(|items| {
        items
            .iter()
            .any(|item| item.kind == ContextLegendKind::Inferred)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ToolAnchor;
    use serde_json::json;

    fn msg(text: &str) -> Message {
        Message {
            id: "m".into(),
            role: "assistant",
            text: text.into(),
            images: vec![],
            context: None,
        }
    }

    fn cmd_tool(input: &str, output: &str, msg_idx: usize) -> Tool {
        Tool {
            id: "t".into(),
            name: "bash".into(),
            kind: Some(ToolKind::Command),
            changes: vec![],
            input: Value::String(input.into()),
            detail: String::new(),
            done: true,
            failed: false,
            output: output.into(),
            images: vec![],
            structured: Value::Null,
            parent_call: None,
            exit_code: Some(0),
            subagents: vec![],
            anchor: ToolAnchor {
                message: msg_idx,
                offset: 0,
            },
            block: 0,
            order: 0,
            expanded: false,
            raw_expanded: false,
            group_expanded: false,
            started: None,
            duration_ms: None,
        }
    }

    #[test]
    fn thread_char_totals_sums_categories_and_drops_pre_compaction_items() {
        let messages = vec![msg(&"x".repeat(5_000)), msg(&"y".repeat(70))];
        let tools = vec![cmd_tool(&"i".repeat(10), &"o".repeat(9_000), 0)];
        let compactions = vec![json!({
            "anchor": { "message": 1, "offset": 0 },
            "result": { "summary": "s".repeat(300) }
        })];
        let totals = thread_char_totals(&messages, &tools, &[], &compactions);
        assert_eq!(totals.messages, 370);
        assert_eq!(totals.tool_calls, 0);
        assert_eq!(totals.tool_output, 0);
    }

    #[test]
    fn estimate_context_breakdown_leaves_remainder_as_inferred_system() {
        let totals = ContextCharTotals {
            messages: 3_500,    // 1000 tokens
            tool_output: 7_000, // 2000 tokens
            ..Default::default()
        };
        let rows = estimate_context_breakdown(&totals, 10_000).unwrap();
        assert_eq!(
            rows,
            vec![
                ContextBreakdownRow {
                    label: "System".into(),
                    tokens: 7_000,
                    kind: BreakdownRowKind::Inferred,
                },
                ContextBreakdownRow {
                    label: "Tool output".into(),
                    tokens: 2_000,
                    kind: BreakdownRowKind::Estimated,
                },
                ContextBreakdownRow {
                    label: "Messages".into(),
                    tokens: 1_000,
                    kind: BreakdownRowKind::Estimated,
                },
            ]
        );
    }

    #[test]
    fn estimate_context_breakdown_scales_down_when_overshooting_reported_total() {
        let totals = ContextCharTotals {
            messages: 7_000,
            tool_output: 7_000,
            ..Default::default()
        };
        let rows = estimate_context_breakdown(&totals, 2_000).unwrap();
        assert_eq!(
            rows.iter().map(|r| r.kind).collect::<Vec<_>>(),
            vec![BreakdownRowKind::Estimated, BreakdownRowKind::Estimated]
        );
        assert_eq!(rows.iter().map(|r| r.tokens).sum::<u64>(), 2_000);
    }

    #[test]
    fn estimate_context_breakdown_returns_none_without_items_or_total() {
        assert!(estimate_context_breakdown(&ContextCharTotals::default(), 10_000).is_none());
        let totals = ContextCharTotals {
            messages: 700,
            ..Default::default()
        };
        assert!(estimate_context_breakdown(&totals, 0).is_none());
    }

    #[test]
    fn claude_context_breakdown_keeps_used_and_buffer_only() {
        let payload = json!({
            "categories": [
                { "name": "System prompt", "tokens": 3_200, "kind": "used" },
                { "name": "Messages", "tokens": 12_500.4, "kind": "used" },
                { "name": "Autocompact buffer", "tokens": 30_000, "kind": "buffer" },
                { "name": "Free space", "tokens": 150_000, "kind": "free" },
                { "name": "Deferred tools", "tokens": 4_000, "kind": "deferred" },
                { "name": "Empty", "tokens": 0, "kind": "used" }
            ]
        });
        let rows = claude_context_breakdown(&payload).unwrap();
        assert_eq!(
            rows,
            vec![
                ContextBreakdownRow {
                    label: "System prompt".into(),
                    tokens: 3_200,
                    kind: BreakdownRowKind::Used,
                },
                ContextBreakdownRow {
                    label: "Messages".into(),
                    tokens: 12_500,
                    kind: BreakdownRowKind::Used,
                },
                ContextBreakdownRow {
                    label: "Autocompact buffer".into(),
                    tokens: 30_000,
                    kind: BreakdownRowKind::Buffer,
                },
            ]
        );
    }

    #[test]
    fn build_context_legend_folds_past_fifth_adds_uncategorized_buffer_and_free() {
        let row = |label: &str, tokens: u64, kind: BreakdownRowKind| ContextBreakdownRow {
            label: label.into(),
            tokens,
            kind,
        };
        let legend = build_context_legend(
            Some(&[
                row("A", 9_000, BreakdownRowKind::Used),
                row("B", 8_000, BreakdownRowKind::Used),
                row("C", 7_000, BreakdownRowKind::Used),
                row("D", 6_000, BreakdownRowKind::Used),
                row("E", 5_000, BreakdownRowKind::Used),
                row("F", 800, BreakdownRowKind::Used),
                row("G", 200, BreakdownRowKind::Used),
                row("Autocompact buffer", 30_000, BreakdownRowKind::Buffer),
            ]),
            Some(100_000),
            36_000,
        )
        .unwrap();
        assert_eq!(
            legend
                .iter()
                .map(|item| (item.label.as_str(), item.tokens, item.kind))
                .collect::<Vec<_>>(),
            vec![
                ("A", 9_000, ContextLegendKind::Used),
                ("B", 8_000, ContextLegendKind::Used),
                ("C", 7_000, ContextLegendKind::Used),
                ("D", 6_000, ContextLegendKind::Used),
                ("E", 5_000, ContextLegendKind::Used),
                ("Other", 1_000, ContextLegendKind::Other),
                ("Buffer", 30_000, ContextLegendKind::Buffer),
                ("Free space", 34_000, ContextLegendKind::Free),
            ]
        );
    }

    #[test]
    fn build_context_legend_shows_sixth_row_directly_and_omits_free_when_over_window() {
        let row = |label: &str, tokens: u64, kind: BreakdownRowKind| ContextBreakdownRow {
            label: label.into(),
            tokens,
            kind,
        };
        let six: Vec<_> = (1..=6)
            .map(|n| row(&format!("R{n}"), n * 1_000, BreakdownRowKind::Estimated))
            .collect();
        let legend = build_context_legend(Some(&six), None, 21_000).unwrap();
        assert_eq!(
            legend.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(),
            vec!["R6", "R5", "R4", "R3", "R2", "R1"]
        );

        let over = build_context_legend(
            Some(&[row("Messages", 12_000, BreakdownRowKind::Used)]),
            Some(10_000),
            12_000,
        )
        .unwrap();
        assert_eq!(
            over.iter().map(|i| i.kind).collect::<Vec<_>>(),
            vec![ContextLegendKind::Used]
        );
    }
}
