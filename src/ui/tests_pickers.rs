use super::*;

#[test]
fn native_model_and_reasoning_pickers_send_real_rpc_commands() {
    let mut rt = runtime(1280.0);
    let (pi, commands, _events) = crate::backend::Pi::test_connection();
    rt.state.current_mut().harness = "pi".into();
    rt.state.current_mut().pi = Some(pi);
    rt.state.current_mut().status = Status::Ready;
    rt.state.current_mut().apply(
        json!({"type":"response","command":"get_available_models","success":true,"data":{"models":[
            {"provider":"cpa","id":"model-a","name":"Model A","reasoning":true,
             "thinkingLevelMap":{"minimal":null,"low":null,"medium":null}}
        ]}}),
    );
    rt.state.current_mut().apply(json!({"type":"response","command":"get_available_thinking_levels","success":true,"data":{"levels":["off","high"]}}));
    rt.frame(0.0);
    press(&mut rt, "model-effort");
    rt.frame(1.0 / 60.0);
    assert!(rt.find("pi-settings-picker").is_some());
    press(&mut rt, "choose-model-0");
    let model = commands.try_recv().unwrap();
    assert_eq!(model["type"], "set_model");
    assert_eq!(model["provider"], "cpa");
    assert_eq!(model["modelId"], "model-a");
    assert!(rt.state.current().configuring);
    rt.state.current_mut().apply(json!({"type":"response","command":"set_model","success":true,"data":{"provider":"cpa","id":"model-a","name":"Model A"}}));
    assert_eq!(commands.try_recv().unwrap()["type"], "get_state");
    assert_eq!(
        commands.try_recv().unwrap()["type"],
        "get_available_thinking_levels"
    );
    assert_eq!(commands.try_recv().unwrap()["type"], "get_session_stats");
    rt.frame(0.0);
    // The pick closed the model picker; the levels are in the reasoning menu.
    assert_eq!(rt.state.settings_picker, None);
    open_picker(&mut rt, SettingsPicker::Thinking);
    press(&mut rt, "choose-thinking-1");
    let thinking = commands.try_recv().unwrap();
    assert_eq!(thinking["type"], "set_thinking_level");
    assert_eq!(thinking["level"], "high");
    rt.state
        .current_mut()
        .apply(json!({"type":"response","command":"set_thinking_level","success":true}));
    assert_eq!(rt.state.current().thinking, "high");
}

/// Claude Code reports aliases (`opus`, `sonnet[1m]`, ...); the catalog turns them into
/// manifest rows, so the tab shows names like "Claude Opus 5.5", keeps reported extras
/// such as `opusplan`, and a pick sends the routed slug, never the display name.
#[test]
fn claude_picker_lists_manifest_names_and_routes_slug_ids_at_all_widths() {
    let reported = crate::claude_catalog::parse_claude_reported_models(&json!([
        {"value":"default","displayName":"Default (recommended)",
         "supportsEffort":true,"supportedEffortLevels":["low","high"]},
        {"value":"opus","displayName":"Opus"},
        {"value":"sonnet","displayName":"Sonnet"},
        {"value":"haiku","displayName":"Haiku"},
        {"value":"opusplan","displayName":""},
        {"value":"sonnet[1m]","displayName":"Sonnet [1m]"},
    ]));
    let catalog = crate::claude_catalog::with_claude_code_model_routes(
        &crate::claude_catalog::bundled_claude_model_catalog(crate::lifecycle::now()),
        &reported,
    );
    let mut rows =
        crate::claude_catalog::resolve_claude_models_for_version(&catalog, Some("9.9.9"));
    rows.extend(crate::claude_catalog::claude_reported_models(
        &catalog, &reported,
    ));
    assert!(
        rows.iter().any(|row| row.slug == "opusplan")
            && !rows
                .iter()
                .any(|row| matches!(row.slug.as_str(), "default" | "opus" | "sonnet" | "haiku")),
        "reported aliases collapse into manifest rows, extras stay: {:?}",
        rows.iter().map(|row| &row.slug).collect::<Vec<_>>()
    );
    for width in [1280.0] {
        let mut rt = runtime(width);
        rt.set_viewport((width, 900.0), 1.0);
        rt.state.claude_catalog.models = rows
            .iter()
            .filter_map(|row| {
                crate::session::AgentModel::from_harness_value(
                    &crate::claude::claude_row_model_value(&catalog, row),
                    "claude",
                )
            })
            .collect();
        rt.state.claude_catalog.installed = true;
        rt.state.choose_harness("claude");
        let (pi, commands, _events) = crate::backend::Pi::test_connection();
        rt.state.current_mut().pi = Some(pi);
        rt.state.current_mut().status = Status::Ready;
        rt.frame(0.0);
        open_picker(&mut rt, SettingsPicker::Model);
        rt.frame(1.0 / 60.0);
        let tree = rt.accessibility_json().to_string();
        for name in [
            "Claude Opus 5.5",
            "Claude Sonnet 5.5",
            "Claude Haiku 5.5",
            "opusplan",
        ] {
            assert!(tree.contains(name), "{width}: {name} missing: {tree}");
        }
        // The aliases Claude Code reported must not shadow the manifest rows.
        let list = rt.find("model-list");
        assert!(list.is_some(), "{width}: model-list missing");
        // "Claude Opus 5.5" is the first catalog row; picking it sends the routed id.
        press(&mut rt, "choose-model-0");
        let model = commands.try_recv().unwrap();
        assert_eq!(model["type"], "set_model", "{width}");
        assert_eq!(model["provider"], "claude", "{width}");
        assert_eq!(model["modelId"], "claude-opus-5-5", "{width}");
    }
}
/// Opens the model picker or the reasoning menu and draws it.
fn open_picker(rt: &mut Runtime<State>, picker: SettingsPicker) {
    rt.state.open_model_picker(picker);
    rt.invalidate();
    rt.frame(0.0);
}
fn with_levels(rt: &mut Runtime<State>) {
    rt.state.current_mut().apply(json!({"type":"response","command":"get_available_models","success":true,"data":{"models":[
        {"provider":"cpa","id":"gpt-6.1-sol","name":"GPT 6.1 Sol (CPA → Codex)","reasoning":true,
         "thinkingLevelMap":{"off":null,"low":null,"medium":null,"high":null}}
    ]}}));
    let t = rt.state.current_mut();
    t.model = "GPT 6.1 Sol (CPA → Codex)".into();
    t.model_provider = "cpa".into();
    t.model_id = "gpt-6.1-sol".into();
    t.thinking_levels = vec!["off".into(), "low".into(), "medium".into(), "high".into()];
    t.thinking = "medium".into();
    rt.invalidate();
}

/// KSG AntigravityProvider picker: the Antigravity tab shows in the rail and lists
/// its combined `family-level` models at every width.
#[test]
fn antigravity_tab_lists_models_at_all_widths() {
    for width in [1280.0] {
        let mut rt = runtime(width);
        rt.set_viewport((width, 900.0), 1.0);
        // The picker re-reads harnesses.json on open, so the agent lives there.
        std::fs::write(
            rt.state.data_dir.join("harnesses.json"),
            serde_json::json!([{"id":"antigravity","name":"Antigravity",
                "command":"agy","args":["acp"],"env":[]}])
            .to_string(),
        )
        .unwrap();
        rt.state.harnesses = crate::harness::Registry::read();
        let t = rt.state.current_mut();
        t.harness = "antigravity".into();
        t.models = [
            "gemini-3.8-flash-high",
            "gemini-3.8-flash-medium",
            "gemini-3.8-flash-low",
            "gemini-4-carbon",
        ]
        .iter()
        .enumerate()
        .map(|(index, id)| crate::session::AgentModel {
            provider: "antigravity".into(),
            id: (*id).into(),
            name: (*id).into(),
            is_default: index == 0,
            ..Default::default()
        })
        .collect();
        open_picker(&mut rt, SettingsPicker::Model);
        rt.frame(1.0);
        assert!(
            rt.presented("choose-harness-antigravity").is_some(),
            "{width}: Antigravity rail tab"
        );
        press(&mut rt, "choose-harness-antigravity");
        rt.frame(1.0);
        assert!(rt.presented("model-list").is_some(), "{width}");
        for index in 0..4 {
            assert!(
                rt.presented(&format!("choose-model-{index}")).is_some(),
                "{width}: model row {index}"
            );
        }
    }
}

/// KSG's `ModelPickerContent` on `applyCodexModelCatalog` output: the Codex tab
/// shows discovered names, folds manifest legacies behind "Show more", keeps the
/// row the manifest lacks, and carries the update-required notice.
#[test]
fn codex_tab_lists_wire_names_at_all_widths() {
    for width in [1280.0] {
        let mut rt = runtime(width);
        rt.set_viewport((width, 900.0), 1.0);
        // `model/list` rows after `model_value`, including one qualified wire id
        // and one slug the manifest lacks.
        let wire = vec![
            json!({"provider":"codex","id":"gpt-6-astra","name":"GPT-6 Astra Wire",
                "thinkingLevels":["low","medium","high"],"isDefault":true,
                "serviceTiers":[{"id":"default","name":"Standard","isDefault":true},
                                {"id":"fast","name":"Fast","isDefault":false}]}),
            json!({"provider":"codex","id":"zzz-fixture","name":"Fixture Unknown",
                "thinkingLevels":["xhigh"],"contextWindow":32000}),
            json!({"provider":"codex","id":"openai.gpt-6-sol","name":"GPT-6 Sol Wire",
                "thinkingLevels":["high"]}),
            json!({"provider":"codex","id":"gpt-5.5","name":"GPT-5.5 Wire",
                "thinkingLevels":["high"]}),
        ];
        let catalog = crate::codex_catalog::bundled_codex_provider_catalog(crate::lifecycle::now());
        let (merged, update_required) = crate::codex_catalog::apply_codex_model_catalog(
            wire,
            catalog.as_ref(),
            Some("0.150.0"),
        );
        let models: Vec<crate::session::AgentModel> = merged
            .iter()
            .filter_map(|m| crate::session::AgentModel::from_harness_value(m, "codex"))
            .collect();
        rt.state.codex_catalog.models = models.clone();
        rt.state.codex_catalog.update_required_models = update_required;
        let t = rt.state.current_mut();
        t.harness = "codex".into();
        t.models = models;
        open_picker(&mut rt, SettingsPicker::Model);
        rt.frame(1.0);
        press(&mut rt, "choose-harness-codex");
        rt.frame(1.0);
        let tree = rt.accessibility_json().to_string();
        assert!(tree.contains("GPT-6 Astra Wire"), "{width}: {tree}");
        assert!(tree.contains("Fixture Unknown"), "{width}: {tree}");
        // Manifest legacies collapse behind the section toggle until expanded.
        assert!(
            rt.presented("legacy-models-codex").is_some(),
            "{width}: legacy toggle"
        );
        assert!(!tree.contains("GPT-5.5"), "{width}: {tree}");
        // Models the installed CLI predates surface through the existing notice.
        assert!(
            rt.presented("update-required-notice-codex").is_some(),
            "{width}: update-required notice"
        );
        assert!(
            tree.contains("Update the Codex CLI to v0.155.0 or newer"),
            "{width}: {tree}"
        );
        // The toggle may sit below the fold of the model list's clip.
        rt.update(|_, tx| tx.scroll_to_bottom_instant("model-list"));
        rt.frame(0.0);
        press(&mut rt, "legacy-models-codex");
        rt.invalidate();
        rt.frame(1.0);
        let expanded = rt.accessibility_json().to_string();
        assert!(expanded.contains("GPT-6 Sol Wire"), "{width}: {expanded}");
        assert!(expanded.contains("GPT-5.5"), "{width}: {expanded}");
    }
}

/// Short windows used to leave the combined picker taller than the room above the
/// composer, so its rows ran out of the card over the input. Each popover now stays inside
/// the window and its rows inside the card.
#[test]
fn model_and_reasoning_popovers_fit_short_windows_at_all_widths() {
    for width in [620.0, 1280.0] {
        for height in [480.0, 640.0, 900.0] {
            let mut rt = runtime(width);
            rt.set_viewport((width, height), 1.0);
            with_levels(&mut rt);
            rt.frame(0.0);
            for picker in [SettingsPicker::Model, SettingsPicker::Thinking] {
                open_picker(&mut rt, picker);
                rt.frame(1.0);
                let card_key = match picker {
                    SettingsPicker::Model => "pi-settings-picker",
                    SettingsPicker::Thinking => "reasoning-picker",
                };
                let (card, _) = rt
                    .presented(card_key)
                    .unwrap_or_else(|| panic!("{width}x{height}: {card_key}"));
                assert!(
                    card[0] >= 0.0
                        && card[1] >= 0.0
                        && card[0] + card[2] <= width + 0.5
                        && card[1] + card[3] <= height + 0.5,
                    "{width}x{height}: {card_key} leaves the window: {card:?}"
                );
                let inner = match picker {
                    SettingsPicker::Model => ["model-list", "harness-group"],
                    SettingsPicker::Thinking => ["reasoning-body", "reasoning-header"],
                };
                for key in inner {
                    let (rect, _) = rt
                        .presented(key)
                        .unwrap_or_else(|| panic!("{width}x{height}: {key}"));
                    assert!(
                        rect[1] >= card[1] - 0.5 && rect[1] + rect[3] <= card[1] + card[3] + 0.5,
                        "{width}x{height}: {key} runs out of {card_key}: {rect:?} {card:?}"
                    );
                }
                rt.state.settings_picker = None;
                rt.invalidate();
                rt.frame(1.5);
            }
        }
    }
}

#[test]
fn model_picker_supports_favorites_legacy_collapse_unavailable_continuation_and_notices_at_all_widths()
 {
    for width in [1280.0] {
        let mut rt = runtime(width);
        rt.set_viewport((width, 900.0), 1.0);
        rt.state.catalog.models = vec![
            crate::session::AgentModel {
                provider: "cpa".into(),
                id: "claude-opus-5-5".into(),
                name: "Anthropic / Claude Opus 5.5".into(),
                sub_provider: Some("Anthropic".into()),
                badge: Some("new".into()),
                ..Default::default()
            },
            crate::session::AgentModel {
                provider: "cpa".into(),
                id: "claude-opus-4-5".into(),
                name: "Claude Opus 4.5".into(),
                is_legacy: true,
                ..Default::default()
            },
        ];
        rt.state.codex_catalog.models = vec![
            crate::session::AgentModel {
                provider: "codex".into(),
                id: "gpt-6-astra".into(),
                name: "GPT-6 Astra".into(),
                is_default: true,
                ..Default::default()
            },
            crate::session::AgentModel {
                provider: "codex".into(),
                id: "gpt-5.4".into(),
                name: "GPT-5.4".into(),
                is_legacy: true,
                ..Default::default()
            },
        ];
        rt.state.codex_catalog.update_required_models =
            vec![crate::model_picker::UpdateRequiredModel {
                slug: "gpt-6-luna".into(),
                name: "GPT-6 Luna".into(),
                badge: None,
                min_version: "0.155.0".into(),
            }];
        rt.state.current_mut().harness = "pi".into();
        rt.state.current_mut().models = rt.state.catalog.models.clone();
        rt.state.current_mut().model_provider = "cpa".into();
        rt.state.current_mut().model_id = "missing-dynamic".into();
        rt.state.current_mut().model = "missing-dynamic".into();
        rt.state.open_model_picker(SettingsPicker::Model);
        rt.invalidate();
        rt.frame(0.0);

        // Card fits inside viewport
        let (card, _) = rt.presented("pi-settings-picker").unwrap();
        assert!(
            card[0] >= 0.0 && card[0] + card[2] <= width + 0.5 && card[1] >= 0.0,
            "{width}: {card:?}"
        );

        // Favorites tab exists, legacy section collapses behind "Show more", unavailable row badge shows
        assert!(
            rt.presented("choose-harness-favorites").is_some(),
            "{width}"
        );
        assert!(rt.presented("legacy-models-pi").is_some(), "{width}");
        assert!(rt.find("toggle-favorite-0").is_some(), "{width}");
        let tree = rt.accessibility_json().to_string();
        assert!(tree.contains("Unavailable"), "{width}: {tree}");
        assert!(tree.contains("New"), "{width}: {tree}");
        assert!(tree.contains("Show more"), "{width}: {tree}");
        assert!(
            tree.contains("Anthropic") && !tree.contains("Pi · Anthropic"),
            "{width}: {tree}"
        );

        // Expanding legacy shows the legacy model and flips to "Show less"
        press(&mut rt, "legacy-models-pi");
        rt.invalidate();
        rt.frame(0.1);
        let expanded_tree = rt.accessibility_json().to_string();
        assert!(expanded_tree.contains("Claude Opus 4.5"), "{width}");
        assert!(expanded_tree.contains("Show less"), "{width}");

        // Toggling favorite switches and puts model in Favorites tab
        press(&mut rt, "toggle-favorite-0");
        rt.invalidate();
        rt.frame(0.2);
        assert_eq!(rt.state.effective_model_favorites().len(), 1);
        press(&mut rt, "choose-harness-favorites");
        rt.invalidate();
        rt.frame(0.3);
        let fav_tree = rt.accessibility_json().to_string();
        assert!(fav_tree.contains("Claude Opus 5.5"), "{width}: {fav_tree}");

        // Switching to Codex shows update-required notice
        press(&mut rt, "choose-harness-codex");
        rt.invalidate();
        rt.frame(0.4);
        assert!(
            rt.presented("update-required-notice-codex").is_some(),
            "{width}"
        );
        let codex_tree = rt.accessibility_json().to_string();
        assert!(
            codex_tree.contains("Update the Codex CLI to v0.155.0 or newer to use GPT-6 Luna."),
            "{width}: {codex_tree}"
        );

        // Pick GPT-6 Astra on Codex, then mark thread as started: other providers
        // stay reachable through a context handoff instead of being locked out.
        press(&mut rt, "choose-model-0");
        assert_eq!(rt.state.current().harness, "codex");
        assert_eq!(rt.state.current().model_id, "gpt-6-astra");
        rt.state.current_mut().life.latest_user_message_at = Some(1);
        rt.state.open_model_picker(SettingsPicker::Model);
        rt.invalidate();
        rt.frame(0.5);
        assert!(
            find(&rt.accessibility_json(), "choose-harness-pi")
                .and_then(|node| node["states"].as_array().cloned())
                .is_some_and(|states| !states.iter().any(|x| x == "disabled")),
            "{width}: pi stays enabled on a started codex thread"
        );
        // Searching across providers now lists the other provider's models with
        // the context-handoff hint under the name.
        rt.state.model_search.replace_text("opus");
        rt.invalidate();
        rt.frame(0.6);
        let search_tree = rt.accessibility_json().to_string();
        assert!(search_tree.contains("Claude Opus 5.5"), "{width}");
        assert!(
            search_tree.contains("Starts a context handoff"),
            "{width}: {search_tree}"
        );
        // Picking it stages a pending handoff; the harness only moves on send.
        press(&mut rt, "choose-model-0");
        assert_eq!(rt.state.current().harness, "codex");
        assert!(
            rt.state.current().staged_handoff().is_some(),
            "{width}: a pending handoff is staged"
        );

        // On an unstarted thread, searching across providers ranks matches globally
        let t = rt.state.current_mut();
        t.life.latest_user_message_at = None;
        t.handoffs.clear();
        t.staged_handoff = None;
        rt.state.open_model_picker(SettingsPicker::Model);
        rt.invalidate();
        rt.frame(0.7);
        let search_tree = rt.accessibility_json().to_string();
        assert!(search_tree.contains("Claude Opus 5.5"), "{width}");
        assert!(
            !search_tree.contains("Starts a context handoff"),
            "{width}: no handoff hint before the first send"
        );
    }
}

#[test]
fn descriptor_driven_docked_picker_options_ultrathink_lock_and_footer_fit_at_all_widths() {
    for width in [1280.0] {
        let mut rt = runtime(width);
        rt.set_viewport((width, 900.0), 1.0);
        let opus_caps = crate::descriptors::claude_manifest_model_entry("claude-opus-5-5")
            .unwrap()
            .capabilities;
        let acp_caps = crate::descriptors::ModelCapabilities {
            option_descriptors: crate::descriptors::acp_provider_option_descriptors(
                Some(&[
                    json!({"id": "thought_level", "name": "Thinking", "category": "thought_level", "type": "select", "currentValue": "medium", "options": [{"value": "low", "name": "Low"}, {"value": "medium", "name": "Medium"}, {"value": "high", "name": "High"}]}),
                    json!({"id": "auto_approve", "name": "Auto-approve", "type": "boolean", "currentValue": false}),
                ]),
                Some(&json!({
                    "currentModeId": "code",
                    "availableModes": [
                        {"id": "code", "name": "Code"},
                        {"id": "plan", "name": "Plan"}
                    ]
                })),
            ),
        };
        rt.state.claude_catalog.models = vec![crate::session::AgentModel {
            provider: "claude".into(),
            id: "claude-opus-5-5".into(),
            name: "Claude Opus 5.5".into(),
            thinking_levels: vec![
                "low".into(),
                "medium".into(),
                "high".into(),
                "xhigh".into(),
                "max".into(),
            ],
            capabilities: opus_caps,
            ..Default::default()
        }];
        rt.state.claude_catalog.installed = true;
        rt.state.choose_harness("claude");
        rt.state
            .current_mut()
            .set_model("claude".into(), "claude-opus-5-5".into());
        rt.frame(0.0);
        rt.state.open_model_picker(SettingsPicker::Model);
        rt.invalidate();
        rt.frame(0.05);

        // Opus has only reasoning and speed, so the model picker is the list alone and
        // carries no reasoning, speed or permission controls.
        let (card, _) = rt.presented("pi-settings-picker").unwrap();
        assert!(
            card[0] >= 0.0 && card[0] + card[2] <= width + 0.5 && card[1] >= 0.0,
            "{width}: {card:?}"
        );
        for key in [
            "model-picker-options",
            "effort-slider",
            "picker-footer",
            "fast-mode-picker",
            "overflow-access-full",
        ] {
            assert!(
                rt.presented(key).is_none(),
                "{width}: {key} in model picker"
            );
        }

        // Fast/Standard and Ultracode sit under the levels in the reasoning menu.
        rt.state.open_model_picker(SettingsPicker::Thinking);
        rt.invalidate();
        rt.frame(0.08);
        let (menu, _) = rt.presented("reasoning-picker").unwrap();
        let (slider, _) = rt.presented("effort-slider").unwrap();
        let (speed, _) = rt.presented("picker-footer").unwrap();
        assert!(
            menu[0] >= 0.0 && menu[0] + menu[2] <= width + 0.5 && menu[1] >= 0.0,
            "{width}: {menu:?}"
        );
        assert!(
            speed[1] >= slider[1] + slider[3],
            "{width}: speed sits under the levels: {slider:?} {speed:?}"
        );
        assert!(rt.presented("fast-mode-picker").is_some(), "{width}");
        assert!(rt.presented("ultracode-toggle").is_some(), "{width}");
        press(&mut rt, "fast-mode-on");
        press(&mut rt, "ultracode-toggle");
        rt.invalidate();
        rt.frame(0.1);
        assert!(
            rt.state
                .current()
                .model_options
                .iter()
                .any(|o| o.id == "fastMode"
                    && o.value == crate::model_selection::OptionValue::Bool(true)),
            "{width}: fastMode enabled"
        );
        assert!(
            rt.state
                .current()
                .model_options
                .iter()
                .any(|o| o.id == "ultracode"
                    && o.value == crate::model_selection::OptionValue::Bool(true)),
            "{width}: ultracode enabled"
        );

        // Typing "ultrathink" in prompt body locks effort slider and shows note
        rt.state
            .current_mut()
            .draft
            .replace_text("Please ultrathink about this");
        rt.invalidate();
        rt.frame(0.2);
        assert!(rt.presented("ultrathink-lock-note").is_some(), "{width}");
        assert_eq!(
            find(&rt.accessibility_json(), "ultrathink-lock-note").and_then(|n| n["name"].as_str()),
            Some(crate::descriptors::ULTRATHINK_IN_BODY_NOTE),
            "{width}"
        );

        // ACP model shows select and boolean option rows in the model picker's options
        // column, beside the list or, below 640 px, under it.
        rt.state.current_mut().draft.replace_text("");
        rt.state.harnesses.agents = vec![crate::harness::Agent {
            id: "devin".into(),
            name: "Devin".into(),
            command: "devin".into(),
            args: vec![],
            env: vec![],
        }];
        rt.state.choose_harness("devin");
        rt.state.current_mut().draft_model_state = Default::default();
        rt.state.current_mut().models = vec![crate::session::AgentModel {
            provider: "devin".into(),
            id: "fusion".into(),
            name: "Fusion".into(),
            thinking_levels: vec!["low".into(), "medium".into(), "high".into()],
            capabilities: acp_caps,
            ..Default::default()
        }];
        rt.state
            .current_mut()
            .set_model("devin".into(), "fusion".into());
        // Set directly: opening refreshes the catalogs, which drops this fixture agent.
        rt.state.settings_picker = Some(SettingsPicker::Model);
        rt.invalidate();
        rt.frame(0.3);
        let (card, _) = rt.presented("pi-settings-picker").unwrap();
        let (list, _) = rt.presented("model-list").unwrap();
        let (options, _) = rt.presented("model-picker-options").unwrap();
        assert!(
            card[0] >= 0.0 && card[0] + card[2] <= width + 0.5 && card[1] >= 0.0,
            "{width}: {card:?}"
        );
        assert!(
            options[0] + options[2] <= card[0] + card[2] + 0.5
                && options[1] + options[3] <= card[1] + card[3] + 0.5,
            "{width}: options column inside the card: {options:?} {card:?}"
        );
        if width > 640.0 {
            assert!(
                options[0] >= list[0] + list[2] - 1.0,
                "{width}: options column sits beside list: {list:?} {options:?}"
            );
        } else {
            assert!(
                options[1] >= list[1] + list[3] - 1.0,
                "{width}: options column stacks below list: {list:?} {options:?}"
            );
        }
        assert!(
            rt.presented("option-menu-_t3/session-mode").is_some(),
            "{width}"
        );
        assert!(
            rt.presented("option-switch-auto_approve").is_some(),
            "{width}"
        );
        press(&mut rt, "option-switch-auto_approve");
        rt.invalidate();
        rt.frame(0.4);
        assert!(
            rt.state
                .current()
                .model_options
                .iter()
                .any(|o| o.id == "auto_approve"
                    && o.value == crate::model_selection::OptionValue::Bool(true)),
            "{width}"
        );
        press(&mut rt, "option-menu-_t3/session-mode");
        rt.invalidate();
        rt.frame(0.5);
        press(&mut rt, "option-choice-_t3/session-mode-plan");
        rt.invalidate();
        rt.frame(0.6);
        assert!(
            rt.state
                .current()
                .model_options
                .iter()
                .any(|o| o.id == "_t3/session-mode"
                    && o.value == crate::model_selection::OptionValue::Str("plan".into())),
            "{width}"
        );
    }
}

#[test]
fn effort_slider_moves_by_keyboard_click_and_drag() {
    let mut rt = runtime(1280.0);
    with_levels(&mut rt);
    rt.frame(0.0);
    press(&mut rt, "composer-effort");
    rt.frame(1.0);
    let focused = |rt: &Runtime<State>| {
        rt.elements()
            .into_iter()
            .any(|element| element.key.as_deref() == Some("effort-slider") && element.focused)
    };
    for _ in 0..40 {
        if focused(&rt) {
            break;
        }
        rt.key(rune::KeyEvent {
            key: rune::Key::Tab,
            mods: Default::default(),
        });
        rt.frame(1.0);
    }
    assert!(focused(&rt), "Tab reaches the effort slider");
    rt.key(rune::KeyEvent {
        key: rune::Key::Right,
        mods: Default::default(),
    });
    rt.frame(1.5);
    assert_eq!(rt.state.current().thinking, "high");
    rt.key(rune::KeyEvent {
        key: rune::Key::Home,
        mods: Default::default(),
    });
    rt.frame(1.5);
    assert_eq!(rt.state.current().thinking, "off");
    rt.frame(2.0);
    press(&mut rt, "choose-thinking-2");
    assert_eq!(rt.state.current().thinking, "medium");
    rt.frame(3.0);
    let (track, _) = rt.presented("effort-slider").unwrap();
    let y = track[1] + track[3] / 2.0;
    rt.pointer_down(track[0] + 4.0, y);
    rt.pointer_move(track[0] + track[2] * 0.3, y);
    rt.frame(3.5);
    let held = rt.state.thinking_drag.expect("the drag holds the thumb");
    assert!(
        (0.2..0.4).contains(&held.frac),
        "the thumb follows the pointer, not the nearest stop: {held:?}"
    );
    rt.pointer_move(track[0] + track[2] - 4.0, y);
    rt.pointer_up(track[0] + track[2] - 4.0, y);
    rt.frame(4.0);
    assert_eq!(rt.state.current().thinking, "high");
    assert_eq!(rt.state.thinking_drag, None);
    assert!(rt.state.settings_picker.is_some(), "the popover stays open");
}

/// The thumb's centre x for a level index inside the slider's rect.
fn thumb_center(track: [f32; 4], count: usize, index: usize) -> f32 {
    let cap = SLIDER_H / 2.0;
    track[0] + cap + (index as f32 / (count - 1) as f32) * (track[2] - 2.0 * cap)
}

/// A rejected effort level drops its pin instead of staying selected, and a newer
/// pick made during the same flight still lands after the rejection.
#[test]
fn a_rejected_effort_level_does_not_stay_selected() {
    let mut rt = runtime(1280.0);
    let (pi, commands, _events) = crate::backend::Pi::test_connection();
    rt.state.current_mut().harness = "pi".into();
    rt.state.current_mut().pi = Some(pi);
    rt.state.current_mut().status = Status::Ready;
    rt.state.current_mut().apply(json!({"type":"response","command":"get_available_thinking_levels","success":true,"data":{"levels":["off","low","medium","high"]}}));
    rt.state.current_mut().model_id = "model-a".into();
    rt.state.current_mut().model_provider = "cpa".into();
    rt.state.current_mut().thinking = "low".into();
    rt.frame(0.0);
    press(&mut rt, "composer-effort");
    settle(&mut rt);
    let (track, _) = rt.presented("effort-slider").unwrap();
    let y = track[1] + track[3] / 2.0;

    let x = thumb_center(track, 4, 2);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.2);
    assert_eq!(commands.try_recv().unwrap()["level"], "medium");
    // Rejected: the pin clears, the slider shows what the provider still runs.
    rt.state.current_mut().apply(
        json!({"type":"response","command":"set_thinking_level","success":false,"error":"Unsupported level"}),
    );
    rt.invalidate();
    rt.frame(0.2);
    assert_eq!(rt.state.current().selected_thinking, None);
    assert_eq!(rt.state.current().thinking, "low");
    assert_eq!(
        find(&rt.accessibility_json(), "effort-title").and_then(|n| n["name"].as_str()),
        Some("Low")
    );

    // A pick during the next flight, followed by a rejection of the older one, still
    // reconciles the newer wanted level.
    let x = thumb_center(track, 4, 2);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.2);
    assert_eq!(commands.try_recv().unwrap()["level"], "medium");
    let x = thumb_center(track, 4, 3);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.2);
    assert!(
        commands.try_recv().is_err(),
        "the second pick waits for the first ack"
    );
    rt.state.current_mut().apply(
        json!({"type":"response","command":"set_thinking_level","success":false,"error":"Unsupported level"}),
    );
    rt.frame(0.2);
    let mut drain = Vec::new();
    while let Ok(command) = commands.try_recv() {
        drain.push(command);
    }
    let follow = drain
        .iter()
        .find(|command| command["type"] == "set_thinking_level")
        .expect("the newer pick survives the older one's rejection");
    assert_eq!(follow["level"], "high", "{drain:?}");
    rt.state
        .current_mut()
        .apply(json!({"type":"response","command":"set_thinking_level","success":true}));
    assert_eq!(rt.state.current().thinking, "high");
}

#[test]
fn composer_attachment_control_adds_and_removes_named_chips_without_losing_text() {
    let mut rt = runtime(1280.0);
    rt.frame(0.0);
    assert!(rt.find("attach-image").is_some());
    rt.state.current_mut().draft.replace_text("Keep this text");
    rt.state.attach_image("pasted.png", &ui_png(1));
    rt.frame(0.0);
    let id = rt.state.current().draft_images[0].id.clone();
    assert!(rt.find(&format!("attachment-{id}")).is_some());
    assert!(rt.find(&format!("attachment-thumb-{id}")).is_some());
    assert_eq!(rt.state.current().draft.text, "Keep this text pasted.png ");
    assert_eq!(rt.state.current().draft.pills().count(), 1);
    press(&mut rt, &format!("remove-attachment-{id}"));
    press(&mut rt, "confirm-remove-attachment");
    assert!(rt.state.current().draft_images.is_empty());
    assert_eq!(rt.state.current().draft.text, "Keep this text");
    assert_eq!(rt.state.current().draft.pills().count(), 0);
    assert!(rt.find(&format!("attachment-{id}")).is_none());
}

#[test]
fn paste_attaches_clipboard_images_without_inserting_placeholder_text() {
    let mut rt = runtime(1280.0);
    rt.frame(0.0);
    press(&mut rt, "composer-input");
    let image = rune::ClipboardImage::from_png(ui_png(1)).unwrap();
    rt.paste_content(rune::ClipboardContent::new("").with_image(image));
    rt.frame(0.0);
    assert_eq!(rt.state.current().draft_images.len(), 1);
    assert!(rt.state.current().draft.text.is_empty());
    assert!(rt.state.current().has_draft_content());
    assert!(rt.find("send-message").is_some());
    let id = rt.state.current().draft_images[0].id.clone();
    assert!(rt.find(&format!("attachment-{id}")).is_some());
}

#[test]
fn long_text_paste_uses_an_attachment_chip_and_undo_preserves_the_draft() {
    let mut rt = runtime(960.0);
    rt.frame(0.0);
    rt.state.current_mut().draft.replace_text("unsent");
    rt.state.current_mut().draft.set_caret(6);
    press(&mut rt, "composer-input");
    let text = "猫".repeat(crate::composer_paste::THRESHOLD / 3 + 1);
    rt.paste(text.clone());
    rt.frame(0.0);
    assert_eq!(rt.state.current().draft.text, "unsent pasted-text.txt ");
    assert_eq!(rt.state.current().draft.pills().count(), 1);
    let attachment = &rt.state.current().draft_images[0];
    assert_eq!(attachment.name, "pasted-text.txt");
    assert_eq!(
        std::fs::read_to_string(attachment.path.as_ref().unwrap()).unwrap(),
        text
    );
    // Inline file sits in the editor rather than duplicating in the tray.
    assert!(rt.find(&format!("attachment-{}", attachment.id)).is_none());
    rt.key(rune::KeyEvent {
        key: rune::Key::Command('z'),
        mods: rune::Mods {
            cmd: true,
            ..Default::default()
        },
    });
    rt.frame(0.0);
    assert!(rt.state.current().draft_images.is_empty());
    assert_eq!(rt.state.current().draft.text, "unsent");
}

#[test]
fn dropping_an_image_file_attaches_it_from_the_composer() {
    let mut rt = runtime(1280.0);
    rt.frame(0.0);
    let path = std::env::temp_dir().join(format!(
        "shina-ui-drop-{}-{}.png",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::write(&path, ui_png(2)).unwrap();
    let (rect, _) = rt.presented("composer").unwrap();
    rt.drop_files(
        vec![path.clone()],
        Some((rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0)),
    );
    rt.frame(0.0);
    let _ = std::fs::remove_file(&path);
    assert_eq!(rt.state.current().draft_images.len(), 1);
    assert_eq!(
        rt.state.current().draft_images[0].name,
        path.file_name().unwrap().to_string_lossy().into_owned()
    );
}

#[test]
fn new_thread_centres_the_composer_with_the_headline_above_it() {
    for width in [620.0, 1280.0] {
        let mut rt = runtime(width);
        rt.set_viewport((width, 900.0), 1.0);
        rt.frame(0.0);
        rt.frame(1.0);
        let (pane, _) = rt.presented("chat-pane").unwrap();
        let (card, _) = rt.presented("composer").unwrap();
        let (heading, _) = rt.presented("empty-heading").unwrap();
        let (project, _) = rt.presented("draft-project").unwrap();
        let centre = card[1] + card[3] / 2.0;
        let pane_centre = pane[1] + pane[3] / 2.0;
        assert!(
            (centre - pane_centre).abs() < 60.0,
            "composer is off centre at {width}: {card:?} in {pane:?}"
        );
        assert!(
            heading[1] + heading[3] <= card[1] && card[1] - (heading[1] + heading[3]) < 40.0,
            "headline must sit just above the composer at {width}: {heading:?} {card:?}"
        );
        for rect in [heading, project] {
            assert!(
                rect[0] >= pane[0] - 0.5 && rect[0] + rect[2] <= pane[0] + pane[2] + 0.5,
                "headline overflows the pane at {width}: {rect:?} in {pane:?}"
            );
        }
        assert!(rt.find("suggest-review").is_none(), "no starter prompts");
        assert!(
            rt.find("context-usage-ring").is_none(),
            "no empty ring on a fresh draft"
        );
    }
}

#[test]
fn sidebar_lists_pinned_and_active_cards_without_workspace_headers() {
    for width in [1280.0] {
        let mut rt = runtime(width);
        rt.state.populate_demo();
        rt.state.compact_sidebar = width < 760.0;
        rt.frame(0.0);
        rt.frame(1.0);

        // Captain rows stay 56 pt tall and separate from the flat thread list.
        let mut prev_bottom = 0.0f32;
        for i in rt.state.sidebar_captains() {
            let id = rt.state.threads[i].id;
            let r = rect(&rt, &format!("thread-{id}"));
            assert!(
                (r[3] - 56.0).abs() < 0.5,
                "{width}: captain {id} height {r:?}"
            );
            assert!(
                r[1] >= prev_bottom - 0.5,
                "{width}: overlap at captain {id}"
            );
            prev_bottom = r[1] + r[3];
        }

        let heading = rect(&rt, "threads-heading");
        let pinned = rect(&rt, "thread-3");
        let active = rect(&rt, "thread-1");
        assert!(heading[1] < pinned[1] && pinned[1] < active[1], "{width}");
        assert_eq!(active[3], 60.0, "{width}");
        assert!(rt.elements().iter().all(|e| {
            !e.key
                .as_deref()
                .is_some_and(|key| key.starts_with("sidebar-group-"))
        }));
        assert_eq!(
            element_text(&rt, "thread-workspace-3").as_deref(),
            Some("rune")
        );
        assert_eq!(
            element_text(&rt, "thread-workspace-1").as_deref(),
            Some("shina")
        );
        rt.update(|s, _| s.set_project_scope(Some(1), false));
        settle(&mut rt);
        assert!(
            rt.find("thread-1").is_some() && rt.find("thread-3").is_none(),
            "{width}"
        );
        rt.update(|s, _| s.set_project_scope(Some(2), true));
        settle(&mut rt);
        assert!(
            rt.find("thread-1").is_some() && rt.find("thread-3").is_some(),
            "{width}"
        );
    }
    // Unread failed captain shows Failed (not unread blue dot), and needs-you captain
    // shows count pill without any visible "Needs you" status word.
    let mut rt = runtime(1280.0);
    rt.state.populate_demo();
    let homes: Vec<usize> = rt
        .state
        .sidebar_captains()
        .iter()
        .map(|&i| rt.state.threads[i].id)
        .collect();
    let cap0 = rt
        .state
        .threads
        .iter()
        .position(|t| t.id == homes[0])
        .unwrap();
    rt.state.threads[cap0].status = Status::Failed;
    rt.state.threads[cap0].life.last_completed_at = Some(lifecycle::now());
    rt.state.threads[cap0].life.last_visited_at = None;
    let cap1 = rt
        .state
        .threads
        .iter()
        .position(|t| t.id == homes[1])
        .unwrap();
    rt.state.threads[cap1].status = Status::Waiting;
    rt.state.threads[cap1].request = Some(json!({"method":"confirm"}));
    rt.invalidate();
    rt.frame(1.0);
    assert!(
        rt.presented(&format!("captain-unread-{}", homes[0]))
            .is_none()
    );
    assert!(
        rt.presented(&format!("captain-attention-{}", homes[1]))
            .is_some()
    );
    let status_text =
        element_text(&rt, &format!("captain-status-{}", homes[1])).unwrap_or_default();
    assert!(!status_text.contains("Needs you"), "{status_text}");

    // Contrast check on bg and row in both themes for sidebar text tokens.
    for dark in [false, true] {
        let p = palette(dark);
        for bg in [p.bg, p.row] {
            for (ink, floor) in [(p.ink, 4.5), (p.muted, 4.3), (p.faint, 3.0)] {
                let ratio = contrast(ink, bg);
                assert!(ratio >= floor, "dark {dark}: sidebar contrast {ratio:.2}:1");
            }
        }
    }
}

#[test]
fn in_draft_captain_selector_fits_and_opens_on_the_new_thread_route_at_all_widths() {
    for width in [1280.0] {
        let mut rt = runtime(width);
        rt.state.captains = crate::captains::Captain::presets();
        rt.frame(0.0);

        let (trigger, _) = rt
            .presented("captain-selector")
            .expect("captain selector trigger in composer footer");
        assert!(
            trigger[0] >= 0.0 && trigger[0] + trigger[2] <= width + 1.0,
            "{width}: trigger fits {trigger:?}"
        );

        // Clicking the footer trigger opens the in-place selector popover inside the window.
        press(&mut rt, "captain-selector");
        assert!(rt.state.captain_picker);
        rt.frame(0.1);
        let (picker, _) = rt
            .presented("captain-picker")
            .expect("captain picker popover presented");
        assert!(
            picker[0] >= 0.0
                && picker[0] + picker[2] <= width + 1.0
                && picker[1] >= 0.0
                && picker[1] + picker[3] <= 840.0 + 1.0,
            "{width}: picker fits {picker:?}"
        );
        assert!(rt.presented("choose-no-captain").is_some());
        assert!(rt.presented("choose-captain-0").is_some());
        assert!(rt.presented("picker-new-captain").is_some());

        // Sidebar "+" uses the same route: starts/keeps a draft in the workspace with the
        // selector open and search focused (closing it returns focus to the composer).
        press(&mut rt, "captain-selector");
        assert!(!rt.state.captain_picker);
        if width >= 1050.0 {
            press(&mut rt, "assign-captain");
            assert!(rt.state.captain_picker);
            assert!(rt.state.captain_picker_query.focused);
            assert_eq!(rt.state.current().captain, None);
        } else {
            rt.state.open_captain_selector();
            rt.frame(0.1);
            assert!(rt.state.captain_picker);
            assert!(rt.state.captain_picker_query.focused);
            assert_eq!(rt.state.current().captain, None);
        }

        // Filtering by name narrows the list while keeping No captain + New captain.
        rt.state.captain_picker_query.replace_text("Pip");
        rt.invalidate();
        rt.frame(0.1);
        assert!(rt.find("choose-no-captain").is_some());
        assert!(rt.find("choose-captain-0").is_none());
        assert!(rt.find("choose-captain-3").is_some());
        assert!(rt.find("picker-new-captain").is_some());
    }
}

#[test]
fn no_projects_hero_and_add_project_browse_parity_fit_at_all_widths() {
    let root = std::env::temp_dir().join(format!("shina-browse-parity-{}", std::process::id()));
    std::fs::create_dir_all(root.join("alpha-one")).unwrap();
    std::fs::create_dir_all(root.join("alpha-two")).unwrap();
    std::fs::create_dir_all(root.join(".hidden-dir")).unwrap();
    let sep = std::path::MAIN_SEPARATOR;

    for width in [620.0, 1280.0] {
        let mut rt = runtime(width);
        // Remove all workspaces so NoProjectsHero renders on the empty new-thread screen.
        rt.state.projects.entries.clear();
        rt.state.new_thread_in(None);
        rt.invalidate();
        rt.frame(0.0);

        let (hero_btn, _) = rt
            .presented("no-projects-add-workspace")
            .expect("NoProjectsHero Add workspace button");
        assert!(
            hero_btn[0] >= 0.0 && hero_btn[0] + hero_btn[2] <= width + 1.0,
            "{width}: hero button fits {hero_btn:?}"
        );

        // Configured "Add project starts in" directory seeds the folder browser.
        rt.state.thread_defaults.add_project_base_directory = root.to_string_lossy().into_owned();
        press(&mut rt, "no-projects-add-workspace");
        assert_eq!(rt.state.project_picker, Some(ProjectPicker::AddProject));
        press(&mut rt, "add-local-folder");
        assert!(rt.state.picker_browsing);
        assert_eq!(
            rt.state.picker_query.text,
            format!("{}{sep}", root.display())
        );
        finish_folder_listing(&mut rt);
        rt.invalidate();
        rt.frame(0.5);

        // Hidden dotfiles are filtered out unless the query leaf starts with '.'.
        assert!(rt.find("browse-0").is_some());
        assert!(rt.find("browse-1").is_some());
        assert!(rt.find("browse-2").is_none());

        // Tab extends to the common prefix ("alpha-") when multiple matches share a prefix.
        rt.state
            .picker_query
            .replace_text(format!("{}{sep}al", root.display()));
        finish_folder_listing(&mut rt);
        rt.invalidate();
        rt.frame(0.6);
        rt.key(rune::KeyEvent {
            key: rune::Key::Tab,
            mods: rune::Mods::default(),
        });
        rt.frame(0.65);
        assert_eq!(
            rt.state.picker_query.text,
            format!("{}{sep}alpha-", root.display())
        );

        // Typing '.' reveals hidden directories.
        rt.state
            .picker_query
            .replace_text(format!("{}{sep}.", root.display()));
        finish_folder_listing(&mut rt);
        rt.invalidate();
        rt.frame(0.7);
        assert!(rt.find("browse-0").is_some());
    }

    let _ = std::fs::remove_dir_all(root);
}
