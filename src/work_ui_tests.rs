use super::*;
use crate::ui::tests::{press, runtime};

#[test]
fn completed_failed_and_unknown_work_outcomes_remain_distinct() {
    let mut state = State::new(".".into(), false);
    let t = state.current_mut();
    t.apply(json!({"type":"agent_start"}));
    assert_eq!(t.work[0].outcome, Some(crate::model::Status::Running));
    t.apply(json!({"type":"agent_settled"}));
    assert_eq!(t.work[0].outcome, Some(crate::model::Status::Completed));
    t.apply(json!({"type":"agent_start"}));
    t.error = Some("fixture failure".into());
    t.apply(json!({"type":"agent_settled"}));
    assert_eq!(t.work[0].outcome, Some(crate::model::Status::Failed));
    let saved = serde_json::to_value(&t.work[0]).unwrap();
    let restored: crate::model::AssistantWork = serde_json::from_value(saved.clone()).unwrap();
    assert_eq!(restored.outcome, Some(crate::model::Status::Failed));
    let mut legacy = saved;
    legacy.as_object_mut().unwrap().remove("outcome");
    let legacy: crate::model::AssistantWork = serde_json::from_value(legacy).unwrap();
    assert_eq!(legacy.outcome, None);
}

#[test]
fn html_cards_fit_the_lane_cap_the_snapshot_and_switch_theme_at_every_width() {
    for width in [1280.0] {
        for snapshot in [false, true] {
            let reference = crate::html_render::tests::fixture(snapshot);
            let mut rt = runtime(width);
            let t = rt.state.current_mut();
            t.apply(json!({"type":"tool_execution_start","toolCallId":"html","toolName":"html_render","args":{}}));
            t.apply(json!({"type":"tool_execution_end","toolCallId":"html",
                "result":{"content":[],"structuredContent":crate::html_render::result(&reference)}}));
            for dark in [false, true] {
                rt.state.dark = dark;
                rt.frame(0.0);
                rt.frame(1.0);
                let (lane, _) = rt.presented("chat-lane").unwrap();
                let (card, _) = rt.presented("html-card-1-0").unwrap();
                assert!(
                    card[0] >= lane[0] && card[0] + card[2] <= lane[0] + lane[2] + 0.5,
                    "{width}/{snapshot}/{dark}: {card:?} vs {lane:?}"
                );
                if snapshot {
                    let (image, _) = rt.presented("html-snapshot-1-0").unwrap();
                    assert!(image[3] > 0.0 && image[3] <= 480.5, "{image:?}");
                    assert!(rt.find("html-placeholder-1-0").is_none());
                    let (natural, _) = rt.presented("html-snapshot-image-1-0").unwrap();
                    let expected_ratio = reference.snapshot(dark).unwrap().width as f32
                        / reference.snapshot(dark).unwrap().height as f32;
                    assert!(
                        (natural[2] / natural[3] - expected_ratio).abs() < 0.02,
                        "{natural:?}"
                    );
                } else {
                    assert!(rt.find("html-placeholder-1-0").is_some());
                    assert!(rt.find("html-snapshot-1-0").is_none());
                }
                let button = rt
                    .elements()
                    .into_iter()
                    .find(|e| e.key.as_deref() == Some("html-card-1-0"))
                    .unwrap();
                assert_eq!(button.role, "button");
                assert_eq!(button.label.as_deref(), Some("Open Example chart"));
            }
        }
    }
}

#[test]
fn expanded_kinds_fit_the_lane_and_each_output_block_is_capped() {
    use ToolKind::*;
    for width in [1280.0] {
        for kind in [
            Command, FileChange, FileSearch, WebSearch, Read, List, Subagent, Other,
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
            // KSG's WorkLogDetails bounds the whole body at 384 pt (`max-h-96`); its
            // StructuredValue blocks cap at 320 (`max-h-80`).
            assert!(frame[3] <= 384.5, "{width}/{kind:?}: {frame:?}");
            let block = match kind {
                FileChange => None,
                FileSearch => Some(format!("tool-search-results-{suffix}")),
                WebSearch => Some(format!("tool-web-results-{suffix}")),
                _ => Some(format!("tool-text-output-{suffix}")),
            };
            if let Some(key) = block {
                let (rect, _) = rt.presented(&key).unwrap();
                assert!(rect[3] <= 320.5, "{width}/{kind:?}: {rect:?}");
            }
            if kind == FileChange {
                // KSG's file_change inspector: the paths stay in the chat, the diff
                // opens in the diff panel through the "Open diff" button.
                let open = rt
                    .elements()
                    .into_iter()
                    .find(|e| e.key.as_deref() == Some(&format!("tool-open-diff-{suffix}")))
                    .unwrap();
                assert_eq!(open.role, "button");
                let path = rt
                    .elements()
                    .into_iter()
                    .find(|e| e.key.as_deref() == Some(&format!("tool-file-path-{suffix}-0")))
                    .unwrap();
                assert_eq!(path.text.as_deref(), Some("update src/lib.rs"));
                let removed = rt
                    .elements()
                    .into_iter()
                    .find(|e| e.key.as_deref() == Some(&format!("tool-file-path-{suffix}-1")))
                    .unwrap();
                assert_eq!(removed.text.as_deref(), Some("delete removed.rs"));
            }
            assert!(rt.find(&format!("tool-exit-{suffix}")).is_some());
        }
    }
}

#[test]
fn consecutive_thoughts_share_a_row_and_expand_in_order_at_all_widths() {
    for width in [1280.0] {
        let mut rt = runtime(width);
        let t = rt.state.current_mut();
        t.apply(
            json!({"type":"response","id":"history","command":"get_messages","success":true,
            "data":{"messages":[{"role":"assistant","content":[
                {"type":"thinking","thinking":"First thought."},
                {"type":"thinking","thinking":"Second thought."},
                {"type":"thinking","thinking":"Third thought."},
                {"type":"text","text":"Checking a.rs."},
                {"type":"toolCall","id":"read","name":"read","arguments":{"path":"a.rs"}},
                {"type":"thinking","thinking":"After the tool."},
                {"type":"text","text":" Done."}]}]}}),
        );
        let parts = turn_parts(t, 0, 1);
        assert_eq!(
            parts[0],
            TranscriptPart::Work(vec![
                WorkEntry::Thought(0),
                WorkEntry::Thought(1),
                WorkEntry::Thought(2)
            ])
        );
        assert!(matches!(parts[1], TranscriptPart::Text { .. }));
        // KSG Code's work streak: the call and the thinking after it are one group.
        assert_eq!(
            parts[2],
            TranscriptPart::Work(vec![WorkEntry::Tool(0), WorkEntry::Thought(3)])
        );
        t.work[0].expanded = true;
        let id = t.id;
        rt.frame(0.0);
        rt.frame(1.0);
        assert!(rt.find(&format!("inline-thought-{id}-0")).is_some());
        assert!(rt.find(&format!("inline-thought-{id}-1")).is_none());
        assert!(
            rt.elements()
                .iter()
                .any(|e| e.text.as_deref() == Some("Thought (×3)"))
        );
        // The mixed streak folds under its summary; unfolded, its thought is its own row.
        assert!(rt.find(&format!("inline-thought-{id}-3")).is_none());
        press(&mut rt, &format!("toggle-tool-group-{id}-0"));
        rt.frame(2.0);
        assert!(rt.find(&format!("inline-thought-{id}-3")).is_some());
        // KSG's folded single thought reads the thought's own first line.
        assert!(
            rt.elements()
                .iter()
                .any(|e| e.text.as_deref() == Some("After the tool."))
        );
        press(&mut rt, &format!("inline-thought-{id}-0"));
        rt.frame(2.0);
        let mut previous_y = f32::NEG_INFINITY;
        for index in 0..3 {
            let (rect, _) = rt
                .presented(&format!("thought-output-{id}-{index}"))
                .unwrap();
            assert!(rect[1] > previous_y, "{width}: thought order");
            previous_y = rect[1];
        }
        assert!(rt.find(&format!("thought-output-{id}-3")).is_none());
        assert_eq!(
            rt.elements()
                .iter()
                .filter(|e| e
                    .key
                    .as_deref()
                    .is_some_and(|k| k.starts_with("toggle-work-")))
                .count(),
            1
        );
    }
}

#[test]
fn a_500_record_fold_mounts_no_work_rows_and_shows_deferred_loading_and_error_states() {
    for width in [1280.0] {
        let mut rt = runtime(width);
        let t = rt.state.current_mut();
        for index in 0..500 {
            t.apply(
                json!({"type":"tool_execution_start","toolCallId":format!("work-{index}"),
                "toolName":"read","args":{"path":"file.rs"}}),
            );
            t.apply(
                json!({"type":"tool_execution_end","toolCallId":format!("work-{index}"),
                "result":{"content":[{"type":"text","text":"x".repeat(2048)}]}}),
            );
        }
        t.status = crate::model::Status::Completed;
        t.work[0].outcome = Some(crate::model::Status::Completed);
        t.work[0].expanded = false;
        let snapshot = json!({"seq":1,"state":rt.state.projection()});
        let streams = crate::history::Streams::new(&snapshot);
        let page = streams
            .page_with_work(1, None, 65536, None, true, None)
            .unwrap();
        let bytes = page.to_string().len();
        let tools: Vec<_> = page["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|row| row["field"] == "tools")
            .collect();
        assert_eq!(tools.len(), 1, "{width}");
        assert!(bytes < 16 * 1024, "{width}: {bytes} bytes");
        let mut marker = tools[0]["item"]["deferredWork"].clone();
        marker["sourceItemId"] = tools[0]["sourceItemId"].clone();
        let t = rt.state.current_mut();
        t.tools.truncate(1);
        t.deferred_work = vec![marker];
        rt.frame(0.0);
        assert!(
            rt.find(&format!("work-rows-{}", message_key(rt.state.current(), 0)))
                .is_none(),
            "{width}: folded work rows"
        );
        assert!(rt.find("tool-1-0").is_none(), "{width}: folded tool rows");
        press(&mut rt, "toggle-work-1-0");
        rt.frame(1.0);
        let loading_key = format!("work-loading-{}", message_key(rt.state.current(), 0));
        let loading = rt.presented(&loading_key).expect("work skeleton").0;
        let lane = rt.presented("chat-lane").unwrap().0;
        assert!(
            loading[0] >= lane[0] && loading[0] + loading[2] <= lane[0] + lane[2] + 0.5,
            "{width}"
        );
        rt.state.current_mut().deferred_work[0]["error"] = json!("fixture failure");
        rt.invalidate();
        rt.frame(1.0);
        assert!(
            rt.elements().iter().any(|e| e
                .text
                .as_deref()
                .is_some_and(|text| text.contains("Couldn't load work. Click to retry."))),
            "{width}"
        );
        assert!(rt.find(&loading_key).is_none());
        press(&mut rt, "toggle-work-1-0");
        rt.frame(1.0);
        assert!(
            rt.state.current().work[0].expanded,
            "{width}: retry does not collapse"
        );
        // An older peer has inline records and no deferred descriptor, so no skeleton.
        rt.state.current_mut().deferred_work.clear();
        rt.invalidate();
        rt.frame(1.0);
        assert!(rt.find(&loading_key).is_none());
        assert!(
            rt.find(&format!("work-rows-{}", message_key(rt.state.current(), 0)))
                .is_some()
        );
    }
}

#[test]
fn proposed_plan_actions_and_collapse_toggle_at_narrow_and_wide_widths() {
    let long_markdown = format!(
        "# Long Plan\n\n{}",
        (1..=24)
            .map(|i| format!("Line {i} of the proposed plan"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    for width in [620.0, 1280.0] {
        let mut rt = runtime(width);
        let t = rt.state.current_mut();
        t.apply(json!({
            "type": "harness_update",
            "field": "plan",
            "value": { "planId": "long", "markdown": long_markdown }
        }));
        t.work[0].expanded = true;
        let card_id = format!("{}-0", t.id);
        rt.frame(0.0);
        rt.frame(1.0);

        let actions_key = format!("plan-actions-{card_id}");
        let collapse_key = format!("plan-collapse-{card_id}");
        let (actions_rect, _) = rt.presented(&actions_key).expect("actions button");
        assert!(
            (actions_rect[2] - 28.0).abs() < 0.5,
            "{width}: {actions_rect:?}"
        );
        assert!(
            (actions_rect[3] - 28.0).abs() < 0.5,
            "{width}: {actions_rect:?}"
        );
        assert!(
            rt.presented(&collapse_key).is_some(),
            "{width}: collapse button"
        );

        let collapse_btn = rt
            .elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some(collapse_key.as_str()))
            .expect("collapse element");
        assert_eq!(collapse_btn.label.as_deref(), Some("Expand plan"));
        let text_nodes = |rt: &rune::Runtime<State>| -> Vec<String> {
            rt.elements().into_iter().filter_map(|e| e.text).collect()
        };
        let texts = text_nodes(&rt);
        assert!(
            texts
                .iter()
                .any(|t| t.contains("Line 1 of the proposed plan")),
            "{width}: preview should include first line"
        );
        assert!(
            !texts
                .iter()
                .any(|t| t.contains("Line 24 of the proposed plan")),
            "{width}: collapsed preview should hide line 24"
        );

        press(&mut rt, &collapse_key);
        let collapse_btn = rt
            .elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some(collapse_key.as_str()))
            .expect("collapse element after expand");
        assert_eq!(collapse_btn.label.as_deref(), Some("Collapse plan"));
        let texts = text_nodes(&rt);
        assert!(
            texts
                .iter()
                .any(|t| t.contains("Line 24 of the proposed plan")),
            "{width}: expanded body should show line 24"
        );

        press(&mut rt, &collapse_key);
        let collapse_btn = rt
            .elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some(collapse_key.as_str()))
            .expect("collapse element after re-collapse");
        assert_eq!(collapse_btn.label.as_deref(), Some("Expand plan"));
        let texts = text_nodes(&rt);
        assert!(
            !texts
                .iter()
                .any(|t| t.contains("Line 24 of the proposed plan")),
            "{width}: re-collapsed preview should hide line 24"
        );

        press(&mut rt, &actions_key);
        assert!(
            rt.presented(&format!("plan-copy-{card_id}")).is_some(),
            "{width}"
        );
        assert!(
            rt.presented(&format!("plan-download-{card_id}")).is_some(),
            "{width}"
        );
        assert!(
            rt.presented(&format!("plan-save-{card_id}")).is_some(),
            "{width}"
        );
    }
}
