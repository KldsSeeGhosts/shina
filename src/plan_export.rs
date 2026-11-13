//! Ported from KSG Code's `apps/web/src/proposedPlan.ts`.
use std::sync::LazyLock;

use regex::Regex;

pub fn proposed_plan_title(markdown: &str) -> Option<String> {
    static HEADING: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?m)^\s{0,3}#{1,6}\s+(.+)$").unwrap());
    HEADING
        .captures(markdown)
        .map(|capture| capture[1].trim().to_owned())
        .filter(|title| !title.is_empty())
}

pub fn sanitize_plan_file_segment(input: &str) -> String {
    let lower = input.to_lowercase();
    let stripped: String = lower
        .chars()
        .filter(|&c| {
            !matches!(
                c,
                '`' | '\'' | '"' | '.' | ',' | '!' | '?' | '(' | ')' | '[' | ']' | '{' | '}'
            )
        })
        .collect();
    let mut slug = String::with_capacity(stripped.len());
    let mut in_dash = false;
    for ch in stripped.chars() {
        if ch.is_ascii_lowercase() || ch.is_ascii_digit() {
            slug.push(ch);
            in_dash = false;
        } else if !in_dash {
            slug.push('-');
            in_dash = true;
        }
    }
    let trimmed = slug.trim_matches('-');
    if trimmed.is_empty() {
        "plan".to_owned()
    } else {
        trimmed.to_owned()
    }
}

pub fn build_proposed_plan_markdown_filename(plan_markdown: &str) -> String {
    let title = proposed_plan_title(plan_markdown);
    format!(
        "{}.md",
        sanitize_plan_file_segment(title.as_deref().unwrap_or("plan"))
    )
}

pub fn normalize_plan_markdown_for_export(plan_markdown: &str) -> String {
    format!("{}\n", plan_markdown.trim_end())
}

pub fn can_collapse_plan(plan_markdown: &str) -> bool {
    plan_markdown.chars().count() > 900 || plan_markdown.split('\n').count() > 20
}

pub fn build_collapsed_proposed_plan_preview_markdown(
    plan_markdown: &str,
    max_lines: usize,
) -> String {
    let (_, displayed) = crate::thread_find::plan_parts(plan_markdown);
    let mut preview_lines: Vec<&str> = Vec::new();
    let mut visible_line_count = 0usize;
    let mut has_more_content = false;

    for raw_line in displayed.trim_end().split('\n') {
        let line = raw_line.trim_end_matches('\r').trim_end();
        let is_visible_line = !line.trim().is_empty();
        if is_visible_line && visible_line_count >= max_lines {
            has_more_content = true;
            break;
        }
        preview_lines.push(line);
        if is_visible_line {
            visible_line_count += 1;
        }
    }

    while preview_lines
        .last()
        .is_some_and(|line| line.trim().is_empty())
    {
        preview_lines.pop();
    }

    if preview_lines.is_empty() {
        return proposed_plan_title(plan_markdown)
            .unwrap_or_else(|| "Plan preview unavailable.".to_owned());
    }

    if has_more_content {
        preview_lines.push("");
        preview_lines.push("...");
    }

    preview_lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filename_uses_sanitized_heading_or_fallback() {
        assert_eq!(
            build_proposed_plan_markdown_filename("# Add Plan Actions! (v2) [draft]\n\nBody"),
            "add-plan-actions-v2-draft.md"
        );
        assert_eq!(
            build_proposed_plan_markdown_filename("## `Foo.bar`, 'baz' \"qux\"? {one}!\n\nDetails"),
            "foobar-baz-qux-one.md"
        );
        assert_eq!(
            build_proposed_plan_markdown_filename("No heading here, just steps."),
            "plan.md"
        );
        assert_eq!(
            build_proposed_plan_markdown_filename("# --- ... !!! ---\n\nBody"),
            "plan.md"
        );
    }

    #[test]
    fn normalize_export_trims_trailing_whitespace_and_adds_single_newline() {
        assert_eq!(
            normalize_plan_markdown_for_export("# Plan\n\n- Step 1   \n\n\n"),
            "# Plan\n\n- Step 1\n"
        );
        assert_eq!(normalize_plan_markdown_for_export("   \n\t "), "\n");
        assert_eq!(normalize_plan_markdown_for_export("# Plan"), "# Plan\n");
    }

    #[test]
    fn collapsed_preview_caps_visible_lines_and_strips_trailing_blank_lines() {
        let short = "# Plan\n\n## Summary\n\nLine 1\nLine 2\n";
        assert_eq!(
            build_collapsed_proposed_plan_preview_markdown(short, 10),
            "Line 1\nLine 2"
        );

        let long = format!(
            "# Big Plan\n\n{}",
            (1..=12)
                .map(|i| format!("Line {i}"))
                .collect::<Vec<_>>()
                .join("\n\n")
        );
        let preview = build_collapsed_proposed_plan_preview_markdown(&long, 10);
        assert_eq!(
            preview,
            format!(
                "{}\n\n...",
                (1..=10)
                    .map(|i| format!("Line {i}"))
                    .collect::<Vec<_>>()
                    .join("\n\n")
            )
        );

        assert_eq!(
            build_collapsed_proposed_plan_preview_markdown("# Only Title\n\n## Summary\n", 10),
            "Only Title"
        );
        assert_eq!(
            build_collapsed_proposed_plan_preview_markdown("", 10),
            "Plan preview unavailable."
        );
    }

    #[test]
    fn collapse_threshold_matches_ksg_limits() {
        assert!(!can_collapse_plan("short plan"));
        assert!(!can_collapse_plan(&"a".repeat(900)));
        assert!(can_collapse_plan(&"a".repeat(901)));
        let twenty_lines = vec!["line"; 20].join("\n");
        assert!(!can_collapse_plan(&twenty_lines));
        let twenty_one_lines = vec!["line"; 21].join("\n");
        assert!(can_collapse_plan(&twenty_one_lines));
    }
}
