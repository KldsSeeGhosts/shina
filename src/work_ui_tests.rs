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
    for width in [620.0, 960.0, 1280.0, 1920.0] {
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
    for width in [620.0, 960.0, 1280.0, 1920.0] {
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

#[test]
fn consecutive_thoughts_share_a_row_and_expand_in_order_at_all_widths() {
    for width in [620.0, 960.0, 1280.0, 1920.0] {
        let mut rt = runtime(width);
        let t = rt.state.current_mut();
        t.apply(
            json!({"type":"response","id":"history","command":"get_messages","success":true,
            "data":{"messages":[{"role":"assistant","content":[
                {"type":"thinking","thinking":"First thought."},
                {"type":"thinking","thinking":"Second thought."},
                {"type":"thinking","thinking":"Third thought."},
                {"type":"toolCall","id":"read","name":"read","arguments":{"path":"a.rs"}},
                {"type":"thinking","thinking":"After the tool."}]}]}}),
        );
        assert_eq!(
            turn_parts(t, 0, 1),
            vec![
                TranscriptPart::Thought(vec![0, 1, 2]),
                TranscriptPart::Tools(vec![0]),
                TranscriptPart::Thought(vec![3])
            ]
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
                .any(|e| e.text.as_deref() == Some("Thought ×3"))
        );
        assert!(
            rt.elements()
                .iter()
                .any(|e| e.text.as_deref() == Some("Thought"))
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
    for width in [620.0, 960.0, 1280.0, 1920.0] {
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
            rt.find("work-rows-1-0").is_none(),
            "{width}: folded work rows"
        );
        assert!(rt.find("tool-1-0").is_none(), "{width}: folded tool rows");
        press(&mut rt, "toggle-work-1-0");
        rt.frame(1.0);
        let loading = rt.presented("work-loading-1-0").expect("work skeleton").0;
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
        assert!(rt.find("work-loading-1-0").is_none());
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
        assert!(rt.find("work-loading-1-0").is_none());
        assert!(rt.find("work-rows-1-0").is_some());
    }
}

#[test]
fn find_reveals_a_folded_proposed_plan_and_highlights_its_title_and_body() {
    for width in [620.0, 960.0, 1280.0, 1920.0] {
        let mut rt = runtime(width);
        rt.poll = Some(|s, _, tx| s.poll_thread_find(tx));
        let t = rt.state.current_mut();
        t.apply(json!({"type":"harness_update","field":"plan","value":{
            "planId":"find-plan","markdown":"# Needle proposal\n\n## Summary\n\nUse **needle** in the body."}}));
        t.status = crate::model::Status::Completed;
        t.work[0].expanded = false;
        for index in 0..80 {
            t.messages.push(crate::model::Message {
                id: format!("later-{index}"),
                role: "You",
                text: format!("Later message {index}"),
                images: vec![],
            });
        }
        rt.frame(0.0);
        assert!(rt.find("plan-card-1-0").is_none());
        rt.update(|s, _| s.open_thread_find_on_query("needle".into(), None));
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        assert_eq!(rt.state.thread_find.total_matches, 2, "{width}");
        assert!(rt.state.current().work[0].expanded);
        assert!(rt.presented("plan-card-1-0").is_some(), "{width}");
        assert!(
            rt.elements().iter().any(|element| element.role == "mark"),
            "{width}: title highlight"
        );
        rt.update(|s, _| s.step_thread_find(1));
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        assert_eq!(rt.state.thread_find.active_index, 1);
        assert!(
            rt.elements().iter().any(|element| element.role == "mark"),
            "{width}: body highlight"
        );
    }
}

#[test]
fn plans_fit_at_all_widths_and_updates_do_not_add_a_card() {
    for width in [620.0, 960.0, 1280.0, 1920.0] {
        for markdown in [false, true] {
            let mut rt = runtime(width);
            let t = rt.state.current_mut();
            t.apply(json!({"type":"harness_update","field":"plan","value":{
                "planId":"stable","steps":[{"text":"First step","status":"pending"}]}}));
            let anchor = t.tools[0].anchor;
            t.apply(json!({"type":"tool_execution_start","toolCallId":"later","toolName":"read","args":{"path":"a.rs"}}));
            let source = if markdown {
                json!({"planId":"stable","markdown":"## Proposed plan\n\nUse **Rust** and the existing protocol."})
            } else {
                json!({"planId":"stable","explanation":"An example plan.","steps":[
                    {"text":"First step","status":"completed"},{"text":"A wrapping step ".repeat(10),"status":"in_progress"},
                    {"text":"Last step","status":"pending"}]})
            };
            t.apply(json!({"type":"harness_update","field":"plan","value":source}));
            assert_eq!(t.tools.len(), 2);
            assert_eq!(t.tools[0].anchor, anchor);
            t.work[0].expanded = true;
            let id = t.id;
            rt.frame(0.0);
            rt.frame(1.0);
            let (lane, _) = rt.presented("chat-lane").unwrap();
            let (card, _) = rt.presented(&format!("plan-card-{id}-0")).unwrap();
            assert!(
                card[0] >= lane[0] && card[0] + card[2] <= lane[0] + lane[2] + 0.5,
                "{width}: {card:?}"
            );
            assert_eq!(
                rt.elements()
                    .iter()
                    .filter(|e| e
                        .key
                        .as_deref()
                        .is_some_and(|k| k.starts_with("plan-card-")))
                    .count(),
                1
            );
            let block = if markdown {
                "plan-markdown"
            } else {
                "plan-steps"
            };
            let (rect, _) = rt.presented(&format!("{block}-{id}-0")).unwrap();
            assert!(rect[3] <= 200.5);
            if !markdown {
                for (index, status) in ["Done", "In progress", "Pending"].iter().enumerate() {
                    let status_el = rt
                        .elements()
                        .into_iter()
                        .find(|e| {
                            e.key.as_deref() == Some(&format!("plan-step-status-{id}-0-{index}"))
                        })
                        .unwrap();
                    assert_eq!(status_el.label.as_deref(), Some(*status));
                }
            }
        }
    }
}

#[test]
fn compactions_retries_and_tools_keep_their_order_at_the_same_text_offset() {
    let mut thread = Thread::new(1);
    thread
        .apply(json!({"type":"tool_execution_start","toolCallId":"a","toolName":"read","args":{}}));
    thread.apply(json!({"type":"compaction_start"}));
    thread.apply(json!({"type":"compaction_end","result":{"summary":"Kept facts"}}));
    thread.apply(json!({"type":"auto_retry_start","attempt":1,"maxAttempts":5}));
    thread
        .apply(json!({"type":"tool_execution_start","toolCallId":"b","toolName":"read","args":{}}));
    assert_eq!(
        turn_parts(&thread, 0, 1),
        vec![
            TranscriptPart::Tools(vec![0]),
            TranscriptPart::Compaction(0),
            TranscriptPart::Retry(0),
            TranscriptPart::Tools(vec![1])
        ]
    );
    let anchor = thread.native.compactions[0]["anchor"].clone();
    thread.apply(json!({"type":"auto_retry_start","attempt":2,"maxAttempts":5}));
    thread.apply(json!({"type":"auto_retry_end","success":true}));
    assert_eq!(thread.native.compactions[0]["anchor"], anchor);
    assert_eq!(crate::work::retry_events(&thread).len(), 1);
}

#[test]
fn turn_events_fit_in_place_and_stopped_folds_keep_their_wording_at_all_widths() {
    for width in [620.0, 960.0, 1280.0, 1920.0] {
        let mut rt = runtime(width);
        let t = rt.state.current_mut();
        t.apply(json!({"type":"response","id":"history","command":"get_messages","success":true,"data":{"messages":[
            {"role":"user","content":"Before"},{"role":"assistant","content":"First answer"}]}}));
        t.apply(json!({"type":"compaction_start"}));
        t.apply(json!({"type":"compaction_end","result":{"summary":"Kept decisions"}}));
        t.apply(json!({"type":"message_start","message":{"role":"user","content":"After"}}));
        t.apply(json!({"type":"agent_start"}));
        t.apply(json!({"type":"auto_retry_start","attempt":2,"maxAttempts":5,"errorMessage":"Temporary"}));
        t.work.last_mut().unwrap().expanded = true;
        let id = t.id;
        rt.frame(0.0);
        rt.frame(1.0);
        let y = |rt: &rune::Runtime<State>, key: &str| rt.presented(key).unwrap().0[1];
        assert!(y(&rt, &format!("message-{id}-0")) < y(&rt, "compaction-0"));
        assert!(y(&rt, "compaction-0") < y(&rt, &format!("message-{id}-2")));
        assert!(
            rt.elements()
                .iter()
                .any(|e| e.text.as_deref() == Some("Retrying provider (2/5)"))
        );
        let (lane, _) = rt.presented("chat-lane").unwrap();
        for key in ["compaction-0".to_owned(), format!("provider-retry-{id}-0")] {
            let (rect, _) = rt.presented(&key).unwrap();
            assert!(
                rect[0] >= lane[0] && rect[0] + rect[2] <= lane[0] + lane[2] + 0.5,
                "{width}/{key}: {rect:?}"
            );
        }
        rt.state
            .current_mut()
            .apply(json!({"type":"auto_retry_end","success":true}));
        rt.invalidate();
        rt.frame(2.0);
        rt.frame(3.0);
        assert!(
            rt.elements()
                .iter()
                .any(|e| e.text.as_deref() == Some("Provider recovered"))
        );
        let t = rt.state.current_mut();
        t.apply(json!({"type":"auto_retry_start","attempt":3,"maxAttempts":5}));
        t.apply(
            json!({"type":"auto_retry_end","success":false,"finalError":"Final provider error"}),
        );
        t.apply(json!({"type":"agent_settled","aborted":false}));
        rt.invalidate();
        rt.frame(4.0);
        rt.frame(5.0);
        assert!(
            rt.elements()
                .iter()
                .any(|e| e.text.as_deref() == Some("Final provider error"))
        );
        assert!(
            rt.find("retry-session").is_some(),
            "the existing Retry control remains"
        );
        assert_eq!(
            rt.elements()
                .iter()
                .filter(|e| e
                    .key
                    .as_deref()
                    .is_some_and(|k| k.starts_with("provider-retry-")))
                .count(),
            1
        );
        let t = rt.state.current_mut();
        t.apply(json!({"type":"agent_settled","aborted":true}));
        t.work.last_mut().unwrap().elapsed = Some(134);
        rt.invalidate();
        rt.frame(6.0);
        rt.frame(7.0);
        assert!(
            rt.elements()
                .iter()
                .any(|e| e.text.as_deref() == Some("You stopped after 2m 14s"))
        );
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
fn positional_cards_and_events_before_prose_are_not_moved_below_the_preamble() {
    for kind in ["plan", "compaction", "retry"] {
        let mut rt = runtime(1280.0);
        let t = rt.state.current_mut();
        match kind {
            "plan" => t.apply(json!({"type":"harness_update","field":"plan","value":{"steps":[{"text":"First","status":"pending"}]}})),
            "compaction" => t.apply(json!({"type":"compaction_start"})),
            _ => t.apply(json!({"type":"auto_retry_start","attempt":1})),
        }
        t.apply(json!({"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"After the event."}}));
        t.apply(json!({"type":"tool_execution_start","toolCallId":"later","toolName":"read","args":{"path":"a.rs"}}));
        t.work[0].expanded = true;
        let id = t.id;
        rt.frame(0.0);
        rt.frame(1.0);
        let event_key = match kind {
            "plan" => format!("plan-card-{id}-0"),
            "compaction" => "compaction-0".into(),
            _ => format!("provider-retry-{id}-0"),
        };
        let event = rt.presented(&event_key).unwrap().0;
        let prose = rt.presented(&format!("message-{id}-0-body-0")).unwrap().0;
        assert!(event[1] < prose[1], "{kind} moved after prose");
    }
}
