use super::*;
use crate::ui::tests::{press, runtime};

#[test]
fn expanded_kinds_fit_the_lane_and_each_output_block_is_capped() {
    use ToolKind::*;
    for width in [620.0, 960.0, 1280.0, 1920.0] {
        for kind in [
            Command, FileChange, FileSearch, WebSearch, Read, List, Subagent, Plan, Other,
        ] {
            let mut rt = runtime(width);
            let root = rt.state.working_directory();
            let t = rt.state.current_mut();
            t.apply(json!({"type":"tool_execution_start","toolCallId":"typed","toolName":"opaque",
                "toolKind":kind,"args":{"command":"printf 'hello'","pattern":"needle","query":"Rune Rust","path":"src/lib.rs"}}));
            t.apply(json!({"type":"tool_execution_end","toolCallId":"typed","result":{
                "content":[{"type":"text","text":"long output\n".repeat(100)}],
                "details":{"exitCode":7},"fileSearchResults":[{"fileName":root.join("src/lib.rs"),"line":12,"preview":"let needle = true;"}],
                "webResults":[{"url":"https://example.com","title":"Example result","snippet":"Result snippet"}]}}));
            t.tools[0].changes = vec![
                crate::work::FileChange::new(
                    &root.join("src/lib.rs").to_string_lossy(),
                    "update",
                    &format!("@@ -1 +1 @@\n-old\n+{}\n", "long_diff_line".repeat(100)),
                ),
                crate::work::FileChange::new("removed.rs", "delete", ""),
            ];
            t.tools[0].expanded = true;
            t.work[0].expanded = true;
            rt.frame(0.0);
            rt.frame(1.0);
            let id = rt.state.current().id;
            let suffix = format!("{id}-0");
            let (lane, _) = rt.presented("chat-lane").unwrap();
            let (frame, _) = rt.presented(&format!("tool-output-{suffix}")).unwrap();
            assert!(
                frame[0] >= lane[0] && frame[0] + frame[2] <= lane[0] + lane[2] + 0.5,
                "{width}/{kind:?}: {frame:?}, lane {lane:?}"
            );
            assert!(rt.find(&format!("tool-raw-{suffix}")).is_none());
            let block = match kind {
                Command => "tool-text-output",
                FileChange => "tool-diff",
                FileSearch => "tool-search-results",
                WebSearch => "tool-web-results",
                _ => "tool-text-output",
            };
            let key = if kind == FileChange {
                format!("{block}-{suffix}-0")
            } else {
                format!("{block}-{suffix}")
            };
            let (rect, _) = rt.presented(&key).unwrap();
            assert!(rect[3] <= 200.5, "{width}/{kind:?}: {rect:?}");
            if kind == FileChange {
                let line = rt
                    .elements()
                    .into_iter()
                    .find(|e| e.key.as_deref() == Some(&format!("tool-diff-line-{suffix}-0-2")))
                    .unwrap();
                assert!(
                    line.rect[2] <= rect[2] + 0.5,
                    "{width}: long line widened the block"
                );
                assert!(
                    rt.find(&format!("tool-diff-{suffix}-1")).is_none(),
                    "delete has no fabricated diff"
                );
                let path = rt
                    .elements()
                    .into_iter()
                    .find(|e| e.key.as_deref() == Some(&format!("tool-file-path-{suffix}-0")))
                    .unwrap();
                assert_eq!(path.text.as_deref(), Some("src/lib.rs"));
            }
            assert!(rt.find(&format!("tool-exit-{suffix}")).is_some());
        }
    }
}

#[test]
fn raw_is_a_closed_disclosure_and_web_links_use_rune_open_url() {
    let mut rt = runtime(1280.0);
    let t = rt.state.current_mut();
    t.apply(json!({"type":"tool_execution_start","toolCallId":"web","toolName":"web_search","args":{"query":"Rust"}}));
    t.apply(
        json!({"type":"tool_execution_end","toolCallId":"web","result":{"content":[],
        "webResults":[{"url":"https://example.com/docs","title":"Docs","snippet":"A snippet"},
            {"url":"javascript:alert(1)","title":"Unsafe"}]}}),
    );
    t.tools[0].expanded = true;
    t.work[0].expanded = true;
    let id = t.id;
    rt.frame(0.0);
    rt.frame(1.0);
    assert!(rt.find(&format!("tool-raw-{id}-0")).is_none());
    let link = format!("tool-web-link-{id}-0-0");
    press(&mut rt, &link);
    assert_eq!(rt.take_url_requests(), vec!["https://example.com/docs"]);
    let unsafe_link = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(&format!("tool-web-link-{id}-0-1")))
        .unwrap();
    assert_ne!(unsafe_link.role, "link");
    press(&mut rt, &format!("tool-raw-toggle-{id}-0"));
    assert!(rt.state.current().tools[0].raw_expanded);
    assert!(rt.find(&format!("tool-raw-{id}-0")).is_some());
}
