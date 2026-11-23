use super::*;

#[test]
fn submission_policy_and_prompt_limit_work_at_all_widths() {
    for width in [1280.0] {
        let mut rt = runtime(width);
        rt.frame(0.0);
        press(&mut rt, "composer-input");

        // Under ModEnter, plain Enter inserts a newline instead of sending.
        rt.state.preferences.send_shortcut = crate::composer::SendShortcut::ModEnter;
        rt.state.current_mut().draft.replace_text("hello");
        rt.key(rune::KeyEvent {
            key: rune::Key::Enter,
            mods: Default::default(),
        });
        rt.frame(0.0);
        assert_eq!(rt.state.current().draft.text, "hello\n");
        assert!(rt.state.current().messages.is_empty());

        // Key-repeat Enter under Enter policy does not submit.
        rt.state.preferences.send_shortcut = crate::composer::SendShortcut::Enter;
        rt.state.current_mut().draft.replace_text("hello");
        rt.repeat_key(rune::KeyEvent {
            key: rune::Key::Enter,
            mods: Default::default(),
        });
        rt.frame(0.0);
        assert_eq!(rt.state.current().draft.text, "hello");
        assert!(rt.state.current().messages.is_empty());

        // Oversized prompt blocks send and displays the limit error within the viewport.
        let oversized = "x".repeat(crate::composer::PROVIDER_SEND_TURN_MAX_INPUT_CHARS + 1);
        rt.state.current_mut().draft.replace_text(&oversized);
        rt.key(rune::KeyEvent {
            key: rune::Key::Enter,
            mods: Default::default(),
        });
        rt.frame(0.0);
        assert!(rt.state.current().messages.is_empty());
        assert_eq!(
            rt.state.current().submission_error.as_deref(),
            Some(
                "Prompt is 1 character over the 120,000-character limit. Shorten or split it before sending."
            )
        );
        let (rect, _) = rt
            .presented("composer-prompt-length-error")
            .unwrap_or_else(|| panic!("{width}: composer-prompt-length-error"));
        assert!(
            rect[0] >= 0.0 && rect[0] + rect[2] <= width + 0.5,
            "{width}: {rect:?}"
        );
        assert!(
            rect[1] >= 0.0 && rect[1] + rect[3] <= 720.5,
            "{width}: {rect:?}"
        );
    }
}

#[test]
fn prompt_history_follows_the_carets_visual_line_at_all_widths() {
    for width in [1280.0] {
        let mut rt = runtime(width);
        // An older short prompt, then one that wraps over several lines with no newline.
        let long = "wrapped ".repeat(60).trim_end().to_string();
        rt.state.current_mut().messages = ["short", long.as_str()]
            .into_iter()
            .map(|text| crate::model::Message {
                id: uuid::Uuid::new_v4().to_string(),
                role: "You",
                text: text.into(),
                images: Vec::new(),
                context: None,
            })
            .collect();
        rt.frame(0.0);
        press(&mut rt, "composer-input");
        let mut time = 0.0;
        let mut tap = |rt: &mut Runtime<State>, key: rune::Key| {
            rt.key(rune::KeyEvent {
                key,
                mods: Default::default(),
            });
            // The caret's place reaches the view the frame after it is drawn.
            for _ in 0..3 {
                time += 0.1;
                rt.frame(time);
            }
        };

        tap(&mut rt, rune::Key::Up);
        assert_eq!(rt.state.current().draft.text, long, "{width}: recall");
        let (field, _) = rt.presented("composer-input").expect("composer input");
        // `COMPOSER`'s line height.
        let lines = ((field[3] - 2.0 * COMPOSER_FIELD_PAD) / 20.0).round() as usize;
        assert!(lines >= 2, "{width}: the prompt wraps, {field:?}");

        // The caret starts on the last line. Up climbs the wrapped lines first, and only
        // from the first line recalls the older prompt.
        let mut presses = 0;
        while rt.state.current().draft.text == long && presses < 40 {
            tap(&mut rt, rune::Key::Up);
            presses += 1;
        }
        assert_eq!(rt.state.current().draft.text, "short", "{width}: older");
        assert_eq!(presses, lines, "{width}: one press a line, then the recall");

        // Down from a one-line prompt goes forward, then leaves history from the last line.
        tap(&mut rt, rune::Key::Down);
        assert_eq!(rt.state.current().draft.text, long, "{width}: forward");
        tap(&mut rt, rune::Key::Down);
        assert_eq!(rt.state.current().draft.text, "", "{width}: leaves history");
    }
}

#[test]
fn approval_options_warnings_and_responding_guards_fit_at_supported_widths() {
    for width in [1280.0] {
        let mut rt = runtime(width);
        rt.state.current_mut().apply(json!({
            "type":"extension_ui_request","id":"options","method":"confirm",
            "title":"Allow this command?","message":"Check the requested command.",
            "approvalOptions":[
                {"decision":"decline","label":"Deny"},
                {"decision":"accept","label":"Allow once","warning":"This command changes files."},
                {"decision":"acceptForSession","label":"Allow for this thread","warning":"Future commands can run."},
                {"decision":"cancel","label":"Cancel"}
            ]
        }));
        rt.frame(0.0);
        for key in ["dialog-confirm", "dialog-decline"] {
            let (rect, _) = rt
                .presented(key)
                .unwrap_or_else(|| panic!("{width}: {key}"));
            assert!(
                rect[0] >= 0.0 && rect[0] + rect[2] <= width + 0.5,
                "{width}: {rect:?}"
            );
        }
        assert!(rt.find("dialog-session").is_none());
        let id = rt.state.current().id;
        press(&mut rt, &format!("approval-more-{id}"));
        for key in ["dialog-session", "dialog-cancel"] {
            let (rect, _) = rt
                .presented(key)
                .unwrap_or_else(|| panic!("{width}: {key}"));
            assert!(
                rect[0] >= 0.0 && rect[0] + rect[2] <= width + 0.5,
                "{width}: {rect:?}"
            );
        }
        rt.state.current_mut().responding_request = Some(uuid::Uuid::new_v4());
        rt.invalidate();
        rt.frame(0.0);
        press(&mut rt, "dialog-confirm");
        assert_eq!(
            rt.state.current().request.as_ref().unwrap()["id"],
            "options"
        );
    }
}

#[test]
fn approval_feedback_and_redirect_composer_fit_at_supported_widths() {
    for width in [1280.0] {
        for feedback in [false, true] {
            let mut rt = runtime(width);
            rt.state.current_mut().status = Status::Running;
            rt.state.current_mut().context_window = Some(200_000);
            rt.state.current_mut().apply(json!({
                "type":"extension_ui_request","id":"send-over-approval","method":"confirm",
                "title":"Run this command?","message":"Check the requested command.",
                "responseCapability":{"type":"live","supportsFeedback":feedback}
            }));
            rt.state
                .current_mut()
                .draft
                .replace_text("Use the safer command.");
            rt.frame(0.0);
            press(&mut rt, "composer-input");
            let (rect, _) = rt.presented("send-message").unwrap();
            assert!(
                rect[0] >= 0.0 && rect[0] + rect[2] <= width + 0.5,
                "{width}: {rect:?}"
            );
            press(&mut rt, "send-message");
            assert!(
                rt.state.current().approval_send_pending(),
                "{width}: {feedback}"
            );
            assert_eq!(rt.state.current().draft.text, "Use the safer command.");
            assert!(rt.state.current().request.is_some());
        }
    }
}

#[test]
fn composer_control_shortcuts_open_controls_step_providers_and_jump_models() {
    let mut rt = runtime(1280.0);
    rt.on_key = Some(crate::key);
    {
        let t = rt.state.current_mut();
        t.harness = "pi".into();
        t.models = vec![
            crate::session::AgentModel {
                provider: "cpa".into(),
                id: "model-alpha".into(),
                name: "Alpha (CPA -> Codex)".into(),
                context_window: Some(128_000),
                supports_images: Some(true),
                thinking_levels: vec!["low".into(), "medium".into(), "high".into()],
                service_tiers: vec![],
                ..Default::default()
            },
            crate::session::AgentModel {
                provider: "cpa".into(),
                id: "model-beta".into(),
                name: "Beta (CPA -> Claude)".into(),
                context_window: Some(200_000),
                supports_images: Some(true),
                thinking_levels: vec!["low".into(), "medium".into(), "high".into()],
                service_tiers: vec![],
                ..Default::default()
            },
        ];
        t.model_provider = "cpa".into();
        t.model_id = "model-alpha".into();
        t.model = "Alpha (CPA -> Codex)".into();
        t.thinking = "low".into();
        t.thinking_levels = vec!["low".into(), "medium".into(), "high".into()];
    }
    rt.frame(0.0);

    // Mod+Shift+M opens the model picker, Mod+Shift+M again closes it.
    rt.key(rune::KeyEvent {
        key: rune::Key::Command('m'),
        mods: rune::Mods {
            cmd: true,
            shift: true,
            ..Default::default()
        },
    });
    rt.frame(0.0);
    assert_eq!(rt.state.settings_picker, Some(SettingsPicker::Model));
    assert!(rt.presented("pi-settings-picker").is_some());
    assert!(rt.presented("model-list").is_some());

    // Mod+Shift+ArrowDown steps provider tab in the open model picker.
    rt.key(rune::KeyEvent {
        key: rune::Key::Down,
        mods: rune::Mods {
            cmd: true,
            shift: true,
            ..Default::default()
        },
    });
    rt.frame(0.0);
    assert_eq!(
        rt.state.model_picker_tab,
        Some(crate::model_picker::ModelPickerTab::Instance(
            "codex".into()
        ))
    );

    // Mod+Shift+ArrowUp steps provider tab back.
    rt.key(rune::KeyEvent {
        key: rune::Key::Up,
        mods: rune::Mods {
            cmd: true,
            shift: true,
            ..Default::default()
        },
    });
    rt.frame(0.0);
    assert_eq!(
        rt.state.model_picker_tab,
        Some(crate::model_picker::ModelPickerTab::Instance("pi".into()))
    );

    // Re-populate models after harness switch and jump to the 2nd model with Mod+2.
    {
        let t = rt.state.current_mut();
        t.models = vec![
            crate::session::AgentModel {
                provider: "cpa".into(),
                id: "model-alpha".into(),
                name: "Alpha (CPA -> Codex)".into(),
                context_window: Some(128_000),
                supports_images: Some(true),
                thinking_levels: vec!["low".into(), "medium".into(), "high".into()],
                service_tiers: vec![],
                ..Default::default()
            },
            crate::session::AgentModel {
                provider: "cpa".into(),
                id: "model-beta".into(),
                name: "Beta (CPA -> Claude)".into(),
                context_window: Some(200_000),
                supports_images: Some(true),
                thinking_levels: vec!["low".into(), "medium".into(), "high".into()],
                service_tiers: vec![],
                ..Default::default()
            },
        ];
    }
    rt.frame(0.0);
    rt.key(rune::KeyEvent {
        key: rune::Key::Command('2'),
        mods: rune::Mods {
            cmd: true,
            ..Default::default()
        },
    });
    rt.frame(0.0);
    assert_eq!(rt.state.current().model_id, "model-beta");
    // Without an options column, a pick closes the picker as KSG's does.
    assert_eq!(rt.state.settings_picker, None);
    rt.state.open_model_picker(SettingsPicker::Thinking);
    rt.frame(0.0);

    // Mod+Shift+E closes effort when already open, and opens it when closed.
    rt.key(rune::KeyEvent {
        key: rune::Key::Command('e'),
        mods: rune::Mods {
            cmd: true,
            shift: true,
            ..Default::default()
        },
    });
    rt.frame(0.0);
    assert_eq!(rt.state.settings_picker, None);
    rt.key(rune::KeyEvent {
        key: rune::Key::Command('e'),
        mods: rune::Mods {
            cmd: true,
            shift: true,
            ..Default::default()
        },
    });
    rt.frame(0.0);
    assert_eq!(rt.state.settings_picker, Some(SettingsPicker::Thinking));
    assert!(rt.presented("effort-slider").is_some());

    // Mod+Shift+A opens the access mode picker.
    rt.key(rune::KeyEvent {
        key: rune::Key::Command('a'),
        mods: rune::Mods {
            cmd: true,
            shift: true,
            ..Default::default()
        },
    });
    rt.frame(0.0);
    assert!(rt.state.current().native.access_picker);
    assert!(rt.presented("access-picker").is_some());

    // Custom-bound composer.increaseEffort / decreaseEffort also work.
    rt.state.current_mut().thinking = "low".into();
    crate::keybindings::upsert_custom_rule(
        &mut rt.state.preferences.keybindings,
        "composer.increaseEffort",
        "alt+]",
        None,
        None,
    )
    .unwrap();
    crate::keybindings::upsert_custom_rule(
        &mut rt.state.preferences.keybindings,
        "composer.decreaseEffort",
        "alt+[",
        None,
        None,
    )
    .unwrap();
    rt.key(rune::KeyEvent {
        key: rune::Key::Text("]".into()),
        mods: rune::Mods {
            alt: true,
            ..Default::default()
        },
    });
    rt.frame(0.0);
    assert_eq!(rt.state.current().thinking, "medium");
    rt.key(rune::KeyEvent {
        key: rune::Key::Text("[".into()),
        mods: rune::Mods {
            alt: true,
            ..Default::default()
        },
    });
    rt.frame(0.0);
    assert_eq!(rt.state.current().thinking, "low");

    // Mod+Shift+H opens the machine selector when linked machines exist, and custom
    // composer.cycleHost cycles to the next connected machine.
    connected_machines(&mut rt);
    rt.frame(0.0);
    rt.key(rune::KeyEvent {
        key: rune::Key::Command('h'),
        mods: rune::Mods {
            cmd: true,
            shift: true,
            ..Default::default()
        },
    });
    rt.frame(0.0);
    assert!(rt.state.machines.picker);
    assert!(rt.presented("machine-picker").is_some());

    crate::keybindings::upsert_custom_rule(
        &mut rt.state.preferences.keybindings,
        "composer.cycleHost",
        "alt+h",
        None,
        None,
    )
    .unwrap();
    rt.key(rune::KeyEvent {
        key: rune::Key::Text("h".into()),
        mods: rune::Mods {
            alt: true,
            ..Default::default()
        },
    });
    rt.frame(0.0);
    assert_eq!(rt.state.machines.selected, "studio");

    // Registered but unimplemented controls (workspace, branch, previousWorktree) return false
    // without changing state.
    for ch in ['x', 'g', 'l'] {
        let mut tx = rune::Tx::default();
        assert!(!crate::key(
            &mut rt.state,
            &rune::KeyEvent {
                key: rune::Key::Command(ch),
                mods: rune::Mods {
                    cmd: true,
                    shift: true,
                    ..Default::default()
                },
            },
            &mut tx
        ));
    }
}

#[test]
fn background_send_and_new_thread_chords_work_and_render_toasts_at_all_widths() {
    let enter_chord = |alt: bool| rune::KeyEvent {
        key: rune::Key::Enter,
        mods: rune::Mods {
            cmd: true,
            alt,
            ..Default::default()
        },
    };

    for width in [1280.0] {
        // 1. Draft thread: Mod+Enter and Mod+Alt+Enter start in background and open a fresh focused composer.
        for alt in [false, true] {
            let mut rt = runtime(width);
            rt.on_key = Some(crate::key);
            let (pi, _cmds, _evs) = crate::backend::Pi::test_connection();
            rt.state.current_mut().pi = Some(pi);
            rt.state.current_mut().model_provider = "cpa".into();
            rt.state
                .current_mut()
                .draft
                .replace_text("Draft background");
            rt.frame(0.0);
            press(&mut rt, "composer-input");
            let first_id = rt.state.current().id;

            rt.key(enter_chord(alt));
            rt.frame(0.0);

            assert_ne!(rt.state.current().id, first_id, "{width} alt={alt}");
            assert!(rt.state.current().is_draft());
            assert!(rt.state.current().draft.focused);
            assert!(rt.state.current().draft.text.is_empty());
            assert_eq!(rt.state.threads[0].messages.len(), 1);
            assert_eq!(rt.state.threads[0].status, Status::Running);

            let (stack, _) = rt.presented("toast-stack").expect("toast stack");
            assert!(
                stack[0] >= 0.0 && stack[0] + stack[2] <= width + 1.0,
                "{width}: {stack:?}"
            );
            let toast_action = format!("toast-action-{}", rt.state.toasts[0].id);
            let (btn, _) = rt.presented(&toast_action).expect("Open action");
            assert!(btn[0] >= 0.0 && btn[0] + btn[2] <= width + 1.0);
            press(&mut rt, &toast_action);
            assert_eq!(rt.state.current().id, first_id);
            assert!(rt.state.toasts.is_empty());
        }

        // 2. Existing idle thread: Mod+Alt+Enter sends and opens a fresh focused draft composer.
        {
            let mut rt = runtime(width);
            rt.on_key = Some(crate::key);
            let (pi, _cmds, _evs) = crate::backend::Pi::test_connection();
            {
                let t = rt.state.current_mut();
                t.pi = Some(pi);
                t.model_provider = "cpa".into();
                t.messages.push(crate::model::Message {
                    id: uuid::Uuid::new_v4().to_string(),
                    role: "You",
                    text: "Prior turn".into(),
                    images: vec![],
                    context: None,
                });
                t.draft.replace_text("Follow-up idle");
            }
            let existing_id = rt.state.current().id;
            rt.frame(0.0);
            press(&mut rt, "composer-input");
            rt.key(enter_chord(true));
            rt.frame(0.0);

            assert_ne!(rt.state.current().id, existing_id);
            assert!(rt.state.current().is_draft());
            assert!(rt.state.current().draft.focused);
            assert!(rt.state.current().draft.text.is_empty());
            assert!(rt.state.toasts.is_empty());
            assert_eq!(rt.state.threads[0].messages.len(), 2);
        }

        // 3. Existing running thread: Mod+Alt+Enter queues/steers and opens a fresh focused draft composer.
        {
            let mut rt = runtime(width);
            rt.on_key = Some(crate::key);
            let (pi, _cmds, _evs) = crate::backend::Pi::test_connection();
            {
                let t = rt.state.current_mut();
                t.pi = Some(pi);
                t.model_provider = "cpa".into();
                t.status = Status::Running;
                t.native.follow_up = true;
                t.draft.replace_text("Follow-up while running");
            }
            let running_id = rt.state.current().id;
            rt.frame(0.0);
            press(&mut rt, "composer-input");
            rt.key(enter_chord(true));
            rt.frame(0.0);

            assert_ne!(rt.state.current().id, running_id);
            assert!(rt.state.current().is_draft());
            assert!(rt.state.current().draft.focused);
            assert_eq!(rt.state.threads[0].native.queue.len(), 1);
        }

        // 4. Failing background send: shows "Background task failed" toast with "Open draft" and recovers draft.
        {
            let mut rt = runtime(width);
            rt.on_key = Some(crate::key);
            rt.state.runtime_enabled = true;
            let failed_id = rt.state.current().id;
            rt.state.current_mut().model_provider = "cpa".into();
            rt.state.current_mut().machine = "missing-host".into();
            rt.state.current_mut().draft.replace_text("Recover me");
            rt.frame(0.0);
            press(&mut rt, "composer-input");
            rt.key(enter_chord(false));
            rt.frame(0.0);

            assert_ne!(rt.state.current().id, failed_id);
            let (stack, _) = rt.presented("toast-stack").expect("failure toast");
            assert!(stack[0] >= 0.0 && stack[0] + stack[2] <= width + 1.0);
            let toast_action = format!("toast-action-{}", rt.state.toasts[0].id);
            press(&mut rt, &toast_action);
            assert_eq!(rt.state.current().id, failed_id);
            assert_eq!(rt.state.current().draft.text, "Recover me");
            assert!(rt.state.current().error.is_some());
        }

        // 5. Guards: empty prompt, submission_error, pending question, archived, unfocused, repeat, IME composition.
        {
            let mut rt = runtime(width);
            rt.on_key = Some(crate::key);
            let (pi, _cmds, _evs) = crate::backend::Pi::test_connection();
            rt.state.current_mut().pi = Some(pi);
            rt.state.current_mut().model_provider = "cpa".into();
            rt.frame(0.0);
            press(&mut rt, "composer-input");
            let id = rt.state.current().id;

            // Empty prompt
            rt.key(enter_chord(false));
            rt.frame(0.0);
            assert_eq!(rt.state.current().id, id);
            assert!(rt.state.toasts.is_empty());

            // Unfocused composer does not trigger sendBackground
            rt.state.current_mut().draft.replace_text("Unfocused");
            rt.state.current_mut().draft.focused = false;
            rt.invalidate();
            rt.frame(0.0);
            rt.key(enter_chord(false));
            rt.frame(0.0);
            assert_eq!(rt.state.current().id, id);
            assert_eq!(rt.state.current().draft.text, "Unfocused");

            // Archived thread does not send
            press(&mut rt, "composer-input");
            rt.state.current_mut().life.archived_at = Some(1);
            rt.key(enter_chord(false));
            rt.frame(0.0);
            assert_eq!(rt.state.current().id, id);
            assert_eq!(rt.state.current().draft.text, "Unfocused");
            rt.state.current_mut().life.archived_at = None;

            // A pending question blocks sendAndNewThread. Approvals use the redirect transaction.
            rt.state.current_mut().messages.push(crate::model::Message {
                id: uuid::Uuid::new_v4().to_string(),
                role: "You",
                text: "Turn 1".into(),
                images: vec![],
                context: None,
            });
            rt.state.current_mut().request = Some(json!({"id":"r1","method":"input"}));
            rt.key(enter_chord(true));
            rt.frame(0.0);
            assert_eq!(rt.state.current().id, id);
            assert_eq!(rt.state.current().draft.text, "Unfocused");
            rt.state.current_mut().request = None;

            // Over-limit prompt sets submission_error and does not open a new thread
            rt.state
                .current_mut()
                .draft
                .replace_text(&"x".repeat(crate::composer::PROVIDER_SEND_TURN_MAX_INPUT_CHARS + 1));
            rt.key(enter_chord(true));
            rt.frame(0.0);
            assert_eq!(rt.state.current().id, id);
            assert!(rt.state.current().submission_error.is_some());

            // Key repeat and IME composition never submit
            rt.state.current_mut().submission_error = None;
            rt.state.current_mut().draft.replace_text("Repeat/IME");
            rt.frame(0.0);
            rt.repeat_key(enter_chord(true));
            rt.frame(0.0);
            assert_eq!(rt.state.current().id, id);
            assert_eq!(rt.state.current().draft.text, "Repeat/IME");
            rt.ime(rune::Ime::Preedit("あ".into(), None, Vec::new()));
            rt.frame(0.0);
            assert!(rt.state.current().draft.is_composing());
            rt.key(enter_chord(true));
            rt.frame(0.0);
            assert_eq!(rt.state.current().id, id);
            assert_eq!(rt.state.current().draft.text, "Repeat/IME");
            rt.state.current_mut().draft.cancel_composition();
            rt.frame(0.0);

            // Open slash menu does not intercept Mod+Alt+Enter (sendAndNewThread)
            rt.state.current_mut().draft.replace_text("/review");
            rt.state
                .current_mut()
                .native
                .commands
                .push(crate::model::pi_session::SlashCommand {
                    name: "review".into(),
                    description: "Review".into(),
                    source: "ACP".into(),
                });
            rt.frame(0.0);
            rt.key(enter_chord(true));
            rt.frame(0.0);
            assert_ne!(rt.state.current().id, id);
            assert!(rt.state.current().is_draft());
            assert!(rt.state.current().draft.focused);
        }
    }
}

#[test]
fn mention_menu_shows_model_and_machine_rows_at_620_and_1280_px() {
    for width in [620.0, 1280.0] {
        let mut rt = runtime(width);
        rt.state.peers.push(crate::peer::Peer {
            server_id: "server-b".into(),
            host: "127.0.0.1:47329".into(),
            label: "StudioSol".into(),
            linked_at: 0,
            capabilities: json!({"contracts": [crate::protocol::THREAD_READ]}),
            last_reached_at: None,
            last_error: None,
        });
        rt.state.codex_catalog.models = vec![
            crate::session::AgentModel {
                provider: "codex".into(),
                id: "gpt-6.1-sol".into(),
                name: "GPT-6.1 Sol".into(),
                ..Default::default()
            },
            crate::session::AgentModel {
                provider: "codex".into(),
                id: "gpt-5.0-sol-legacy".into(),
                name: "GPT-5.0 Sol Legacy".into(),
                is_legacy: true,
                ..Default::default()
            },
            crate::session::AgentModel {
                provider: "codex".into(),
                id: "gpt-6.2-sol-unavail".into(),
                name: "GPT-6.2 Sol Unavailable".into(),
                is_unavailable: true,
                ..Default::default()
            },
        ];
        rt.frame(0.0);
        press(&mut rt, "composer-input");
        rt.key(rune::KeyEvent {
            key: rune::Key::Text("@sol".into()),
            mods: Default::default(),
        });
        rt.frame(0.0);
        rt.frame(0.25);
        let (menu, _) = rt.presented("mention-menu").expect("mention menu");
        assert!(
            menu[0] >= 8.0 && menu[0] + menu[2] <= width - 7.0,
            "{width}: {menu:?}"
        );
        assert!(
            rt.presented("mention-item-0").is_some(),
            "{width}: machine row"
        );
        assert!(
            rt.presented("mention-item-1").is_some(),
            "{width}: model row"
        );
        // Legacy and unavailable models are hidden, matching KSG.
        assert!(
            rt.presented("mention-item-2").is_none(),
            "{width}: hidden models"
        );
    }
}

#[test]
fn mention_menu_lists_workspace_files_and_older_threads_at_620_and_1280_px() {
    for width in [620.0, 1280.0] {
        let mut rt = runtime(width);
        let (id, machine) = {
            let t = rt.state.current();
            (t.id, t.machine.clone())
        };
        // Results already applied for the current key, as a settled
        // `workspace.searchEntries`/`threads.search` response leaves them.
        rt.state.mention_search.key = Some((id, machine.clone(), "deep".into()));
        rt.state.mention_search.paths_cwd = "/tmp/shina-mention-fixture".into();
        rt.state.mention_search.paths = vec![crate::workspace_search::WorkspaceEntry {
            path: "src/deep/nested.rs".into(),
            kind: "file".into(),
            ignored: false,
        }];
        rt.state.mention_search.threads = vec![crate::ui::composer_menu::MentionThreadRow {
            server: machine,
            id: 9,
            title: "Deep settings rework".into(),
            branch: None,
            updated_ms: Some(1),
            archived: false,
            parent: false,
        }];
        rt.frame(0.0);
        press(&mut rt, "composer-input");
        rt.key(rune::KeyEvent {
            key: rune::Key::Text("@deep".into()),
            mods: Default::default(),
        });
        rt.frame(0.0);
        rt.frame(0.25);
        let (menu, _) = rt.presented("mention-menu").expect("mention menu");
        assert!(
            menu[0] >= 8.0 && menu[0] + menu[2] <= width - 7.0,
            "{width}: {menu:?}"
        );
        assert!(
            rt.presented("mention-item-0").is_some(),
            "{width}: older thread row"
        );
        assert!(
            rt.presented("mention-item-1").is_some(),
            "{width}: workspace file row"
        );
    }
}

#[test]
fn composer_notice_stack_attaches_above_composer_and_shows_details_on_overflow() {
    use crate::composer_notices::{Group, Notice, NoticeAction, Variant};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    for width in [620.0, 1280.0] {
        let mut rt = runtime(width);
        let clicks = Arc::new(AtomicUsize::new(0));
        let clicks_action = clicks.clone();
        rt.state.composer_notices.push(
            Notice::new("att-long", Variant::Info, "alert", "Attention notice")
                .description(Some(
                    "This description is deliberately very long so that it overflows a single line even on wide composer lanes and triggers the Details popover button across every width under test."
                        .into(),
                ))
                .dismiss_label("Dismiss attention notice"),
        );
        rt.state.composer_notices.push(
            Notice::new("env-first", Variant::Warning, "alert", "Studio is offline")
                .group(Group::Environment)
                .description(Some("This device is offline".into()))
                .actions(vec![NoticeAction::new("Retry", move |_| {
                    clicks_action.fetch_add(1, Ordering::Relaxed);
                })]),
        );
        rt.frame(0.0);
        rt.frame(1.0);

        let (stack_rect, _) = rt.presented("composer-notices").expect("notice stack");
        let (env_rect, _) = rt
            .presented("composer-notice-env-first")
            .expect("environment notice");
        let (att_rect, _) = rt
            .presented("composer-notice-att-long")
            .expect("attention notice");
        let (composer_rect, _) = rt.presented("composer").expect("composer card");

        // Environment notice precedes Attention notice, consecutive cards touch, and the
        // last card touches the composer with no gap at the same lane width.
        assert!(
            env_rect[1] < att_rect[1],
            "{width}: env should be above att"
        );
        assert!(
            (att_rect[1] - (env_rect[1] + env_rect[3])).abs() < 0.5,
            "{width}: gap between notices: env={env_rect:?} att={att_rect:?}"
        );
        assert!(
            (composer_rect[1] - (att_rect[1] + att_rect[3])).abs() < 0.5,
            "{width}: gap before composer: att={att_rect:?} composer={composer_rect:?}"
        );
        assert!(
            (stack_rect[2] - composer_rect[2]).abs() < 0.5,
            "{width}: stack width {stack_rect:?} vs composer {composer_rect:?}"
        );

        // Long description shows Details and opens the details popover; Retry triggers action;
        // Dismiss hides the attention notice.
        assert!(
            rt.presented("composer-notice-details-att-long").is_some(),
            "{width}"
        );
        press(&mut rt, "composer-notice-details-att-long");
        rt.frame(1.0);
        assert!(
            rt.presented("composer-notice-details-popover").is_some(),
            "{width}"
        );
        press(&mut rt, "close-notice-details");
        rt.frame(1.0);
        assert!(
            rt.presented("composer-notice-details-popover").is_none(),
            "{width}"
        );

        press(&mut rt, "composer-notice-action-env-first-0");
        rt.frame(1.0);
        assert_eq!(clicks.load(Ordering::Relaxed), 1, "{width}");

        press(&mut rt, "composer-notice-dismiss-att-long");
        rt.frame(1.0);
        assert!(
            rt.presented("composer-notice-att-long").is_none(),
            "{width}"
        );
    }
}

#[test]
fn composer_prompt_suggestion_tab_and_escape_behavior() {
    let mut rt = runtime(1280.0);
    rt.state.thread_defaults.prompt_suggestions_enabled = true;
    {
        let t = rt.state.current_mut();
        t.harness = "codex".into();
        t.status = Status::Completed;
        t.run_id = Some("run-1".into());
        t.prompt_suggestion = Some(crate::model::PromptSuggestion {
            run_id: "run-1".into(),
            text: "Run cargo test".into(),
        });
        t.draft.focused = true;
    }
    rt.frame(0.0);
    rt.frame(1.0);
    press(&mut rt, "composer-input");
    rt.frame(1.0);

    // Tab inserts the suggestion as editable text with caret at the end, and does not send.
    rt.key(rune::KeyEvent {
        key: rune::Key::Tab,
        mods: Default::default(),
    });
    rt.frame(1.0);
    assert_eq!(rt.state.current().draft.text, "Run cargo test");
    assert_eq!(rt.state.current().draft.focus, "Run cargo test".len());
    assert_eq!(rt.state.current().draft.anchor, "Run cargo test".len());
    assert_eq!(rt.state.current().status, Status::Completed);
    assert!(rt.state.current().messages.is_empty());

    // Tab with the slash menu open keeps the menu's behaviour.
    rt.state.current_mut().apply(json!({
        "type": "response",
        "command": "get_commands",
        "success": true,
        "data": {"commands": [{"name": "skill:review", "description": "Review", "source": "skill"}]}
    }));
    rt.state.current_mut().draft.replace_text("/skill:");
    rt.invalidate();
    rt.frame(1.0);
    assert!(rt.presented("slash-menu").is_some());
    rt.key(rune::KeyEvent {
        key: rune::Key::Tab,
        mods: Default::default(),
    });
    rt.frame(1.0);
    assert_eq!(rt.state.current().draft.text, "/skill:review ");
    assert!(rt.find("slash-menu").is_none());

    // Deleting back to empty shows the suggestion again; Escape dismisses only the suggestion
    // and does not close anything else on that press.
    rt.state.current_mut().draft.replace_text("");
    rt.state.open_thread_find();
    rt.invalidate();
    rt.frame(1.0);
    press(&mut rt, "composer-input");
    rt.frame(1.0);
    assert_eq!(rt.state.offered_prompt_suggestion(), Some("Run cargo test"));
    rt.key(rune::KeyEvent {
        key: rune::Key::Escape,
        mods: Default::default(),
    });
    rt.frame(1.0);
    assert_eq!(rt.state.offered_prompt_suggestion(), None);
    assert!(rt.state.thread_find_active());
    assert_eq!(focused_key(&rt).as_deref(), Some("composer-input"));
}

#[test]
fn reply_set_replace_remove_persists_and_handles_accepted_and_failed_send() {
    let dir = std::env::temp_dir().join(format!(
        "shina-reply-persist-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&dir).unwrap();

    let mut rt = runtime(1280.0);
    rt.state.current_mut().messages = vec![
        crate::model::Message {
            id: "msg-a1".into(),
            role: "Pi",
            text: "First assistant response opening.".into(),
            images: Vec::new(),
            context: None,
        },
        crate::model::Message {
            id: "msg-u2".into(),
            role: "You",
            text: "Follow up".into(),
            images: Vec::new(),
            context: None,
        },
        crate::model::Message {
            id: "msg-a2".into(),
            role: "Pi",
            text: "Second assistant response opening.".into(),
            images: Vec::new(),
            context: None,
        },
    ];
    rt.state.new_thread();
    rt.state.open_kept_tab(0);
    rt.frame(0.0);
    rt.frame(1.0);

    // Set reply on first response, replace with second, then remove.
    press(&mut rt, "reply-message-msg-a1");
    rt.frame(1.0);
    assert!(rt.state.current().draft.focused);
    assert!(rt.state.current().has_draft_content());
    assert_eq!(
        rt.state
            .current()
            .draft_reply_to
            .as_ref()
            .map(|r| r.context_id.as_str()),
        Some("response_msg-a1")
    );

    press(&mut rt, "reply-message-msg-a2");
    rt.frame(1.0);
    assert_eq!(
        rt.state
            .current()
            .draft_reply_to
            .as_ref()
            .map(|r| r.context_id.as_str()),
        Some("response_msg-a2")
    );

    press(&mut rt, "composer-reply-remove");
    rt.frame(1.0);
    assert!(rt.state.current().draft_reply_to.is_none());
    assert!(!rt.state.current().has_draft_content());

    // Set again, persist across thread switch and save/reload in temp dir.
    press(&mut rt, "reply-message-msg-a1");
    rt.frame(1.0);
    rt.state.open_kept_tab(1);
    assert!(rt.state.current().draft_reply_to.is_none());
    rt.state.open_kept_tab(0);
    assert_eq!(
        rt.state
            .current()
            .draft_reply_to
            .as_ref()
            .map(|r| r.context_id.as_str()),
        Some("response_msg-a1")
    );

    let saved_reply = rt.state.current().draft_reply_to.clone().unwrap();
    let mut saver = State::new(dir.clone(), false);
    saver.load_global_for_test();
    saver.current_mut().draft_reply_to = Some(saved_reply.clone());
    saver.checkpoint_and_flush().unwrap();
    let mut reloaded = State::new(dir.clone(), false);
    reloaded.load_global_for_test();
    assert_eq!(reloaded.current().draft_reply_to, Some(saved_reply.clone()));
    let _ = std::fs::remove_dir_all(&dir);

    // Local failed send keeps draft_reply_to.
    {
        let t = rt.state.current_mut();
        t.harness = "pi".into();
        t.model_provider = "direct-disallowed".into();
    }
    rt.state.send();
    assert_eq!(rt.state.current().draft_reply_to, Some(saved_reply.clone()));

    // Remote failed send restores draft_reply_to; remote accepted send clears it.
    let mut remote_rt = runtime(1280.0);
    let (client, _local_rx) =
        crate::client::Client::test_connected(remote_rt.state.data_dir.clone());
    remote_rt.state.client = Some(client);
    remote_rt.state.current_mut().draft_reply_to = Some(saved_reply.clone());
    remote_rt.state.send();
    let cmd_id = remote_rt.state.current().remote_send.as_ref().unwrap().0;
    assert!(remote_rt.state.current().draft_reply_to.is_none());
    assert!(
        remote_rt
            .state
            .current_mut()
            .finish_remote_send(cmd_id, false)
    );
    assert_eq!(
        remote_rt.state.current().draft_reply_to,
        Some(saved_reply.clone())
    );
    remote_rt.state.send();
    let cmd_id_ok = remote_rt.state.current().remote_send.as_ref().unwrap().0;
    assert!(
        remote_rt
            .state
            .current_mut()
            .finish_remote_send(cmd_id_ok, true)
    );
    assert!(remote_rt.state.current().draft_reply_to.is_none());

    // Local accepted send attaches the response reference + context record and clears draft_reply_to.
    let (pi, commands, _) = crate::backend::Pi::test_connection();
    {
        let t = rt.state.current_mut();
        t.pi = Some(pi);
        t.model_provider = "cpa".into();
        t.error = None;
        t.draft.replace_text("");
    }
    rt.state.send();
    assert!(rt.state.current().draft_reply_to.is_none());
    let sent_msg = rt.state.current().messages.last().unwrap();
    assert_eq!(sent_msg.role, "You");
    assert!(sent_msg.text.starts_with(
        "[First assistant response opening.](t3-context://v1/response/response_msg-a1)"
    ));
    assert!(
        sent_msg
            .context
            .as_ref()
            .is_some_and(|c| c.records.contains(&saved_reply))
    );
    let sent_cmd = commands
        .try_iter()
        .find(|cmd| cmd["type"] == "prompt")
        .expect("prompt command dispatched");
    assert!(
        sent_cmd["message"]
            .as_str()
            .unwrap_or("")
            .contains("The user is replying to this earlier response of yours.")
    );
}

/// KSG's `AssistantMessageMeta`: Copy then Reply on the prose's left edge, in one row
/// under the response, and Copy shows its check after a press.
#[test]
fn assistant_response_actions_sit_under_the_prose_at_supported_widths() {
    for width in [1280.0] {
        let mut rt = runtime(width);
        rt.state.current_mut().messages.push(crate::model::Message {
            id: "msg-meta".into(),
            role: "Pi",
            text: "An answer with **actions** under it.".into(),
            images: Vec::new(),
            context: None,
        });
        rt.frame(0.0);
        rt.frame(1.0);
        let (copy, _) = rt.presented("copy-message-msg-meta").expect("copy");
        let (reply, _) = rt.presented("reply-message-msg-meta").expect("reply");
        let (lane, _) = rt.presented("chat-lane").expect("lane");
        assert!((copy[1] - reply[1]).abs() < 0.5, "{width}: one row");
        assert!(copy[0] + copy[2] <= reply[0], "{width}: Copy before Reply");
        assert!(
            (copy[0] - lane[0]).abs() <= 6.0,
            "{width}: the actions start at the prose edge {copy:?} {lane:?}"
        );
        assert_eq!((copy[2], copy[3]), (24.0, 24.0));
        assert!(
            rt.presented("fork-message-msg-meta").is_none(),
            "{width}: no session"
        );
        press(&mut rt, "copy-message-msg-meta");
        rt.frame(0.1);
        assert!(rt.state.copied_message.is_some(), "{width}");
        assert_eq!(
            element_text(&rt, "copy-message-msg-meta").as_deref(),
            Some("Copied"),
            "{width}: the cached turn shows the check"
        );
    }
}

#[test]
fn worktree_toolbar_and_setup_card_layout_at_all_widths() {
    for width in [620.0, 1280.0] {
        let mut rt = runtime(width);
        rt.state.compact_sidebar = width < 760.0;
        rt.state.current_mut().draft.focused = true;
        let project_path = rt
            .state
            .project(rt.state.current().project)
            .unwrap()
            .path
            .to_string_lossy()
            .into_owned();
        rt.state.vcs_branches.insert(
            ("local".into(), project_path),
            crate::model::VcsBranchCacheEntry {
                is_repo: true,
                loaded: true,
                loading: false,
                error: None,
                refs: vec![crate::vcs::VcsRef {
                    name: "main".into(),
                    is_remote: Some(false),
                    remote_name: None,
                    upstream_ref: None,
                    current: true,
                    is_default: true,
                    worktree_path: None,
                }],
            },
        );
        rt.state.current_mut().env_mode = Some(crate::model::ThreadEnvMode::Worktree);
        rt.state.current_mut().base_branch = Some("main".into());
        rt.state.current_mut().worktree_setup = Some(crate::model::WorktreeSetupState {
            phase: crate::model::WorktreeSetupPhase::Failed,
            script_name: Some("Setup".into()),
            script_command: Some("npm ci".into()),
            error: Some("Setup script exited with code 1.".into()),
            output: "line 1\nline 2\nline 3\nline 4".into(),
            exit_code: Some(1),
            timed_out: false,
            duration_ms: 420,
            skipped: false,
        });
        rt.state.current_mut().worktree_setup_output_open = true;
        rt.frame(0.0);
        rt.frame(0.25);

        let (footer_rect, _) = rt
            .presented("composer-footer")
            .unwrap_or_else(|| panic!("{width}: composer-footer"));
        for key in ["env-mode-trigger", "base-branch-trigger"] {
            let (rect, _) = rt
                .presented(key)
                .unwrap_or_else(|| panic!("{width}: {key}"));
            assert!(
                rect[0] >= footer_rect[0] - 0.5
                    && rect[0] + rect[2] <= footer_rect[0] + footer_rect[2] + 0.5
                    && rect[0] + rect[2] <= width + 0.5,
                "{width}: {key} overflowed footer: rect={rect:?} footer={footer_rect:?}"
            );
        }

        let (lane_rect, _) = rt
            .presented("chat-lane")
            .unwrap_or_else(|| panic!("{width}: chat-lane"));
        let (card_rect, _) = rt
            .presented("worktree-setup-card")
            .unwrap_or_else(|| panic!("{width}: worktree-setup-card"));
        assert!(
            card_rect[0] >= lane_rect[0] - 0.5
                && card_rect[0] + card_rect[2] <= lane_rect[0] + lane_rect[2] + 0.5,
            "{width}: setup card outside conversation lane: card={card_rect:?} lane={lane_rect:?}"
        );
        for key in [
            "worktree-setup-retry",
            "worktree-setup-skip",
            "worktree-setup-output",
            "worktree-setup-output-tail",
        ] {
            assert!(rt.presented(key).is_some(), "{width}: missing {key}");
        }
        press(&mut rt, "worktree-setup-output");
        assert!(
            !rt.state.current().worktree_setup_output_open,
            "{width}: toggle should flip state"
        );
        rt.frame(0.5);
        assert!(
            rt.presented("worktree-setup-output-tail").is_none(),
            "{width}: output should collapse"
        );
    }
}

fn setup_multi_model_git_runtime(width: f32) -> rune::Runtime<State> {
    let mut rt = runtime(width);
    let project_path = rt
        .state
        .project(rt.state.current().project)
        .unwrap()
        .path
        .to_string_lossy()
        .into_owned();
    rt.state.vcs_branches.insert(
        ("local".into(), project_path),
        crate::model::VcsBranchCacheEntry {
            is_repo: true,
            loaded: true,
            loading: false,
            error: None,
            refs: vec![crate::vcs::VcsRef {
                name: "main".into(),
                is_remote: Some(false),
                remote_name: None,
                upstream_ref: None,
                current: true,
                is_default: true,
                worktree_path: None,
            }],
        },
    );
    rt.state.catalog.models = vec![
        crate::session::AgentModel {
            provider: "cpa".into(),
            id: "cpa/claude-sonnet-4-5".into(),
            name: "Claude Sonnet 4.5".into(),
            thinking_levels: vec!["low".into(), "high".into()],
            ..Default::default()
        },
        crate::session::AgentModel {
            provider: "cpa".into(),
            id: "cpa/gpt-5".into(),
            name: "GPT-5".into(),
            thinking_levels: vec!["medium".into(), "high".into()],
            ..Default::default()
        },
        crate::session::AgentModel {
            provider: "cpa".into(),
            id: "cpa/gemini-2-5-pro".into(),
            name: "Gemini 2.5 Pro".into(),
            ..Default::default()
        },
        crate::session::AgentModel {
            provider: "cpa".into(),
            id: "cpa/deepseek-r1".into(),
            name: "DeepSeek R1".into(),
            ..Default::default()
        },
    ];
    rt.state.catalog.default_model = Some(rt.state.catalog.models[0].clone());
    rt.state.current_mut().harness = "pi".into();
    rt.state.current_mut().models = rt.state.catalog.models.clone();
    rt.state
        .current_mut()
        .set_model("cpa".into(), "cpa/claude-sonnet-4-5".into());
    rt.state.set_composer_thinking_sticky("high".into());
    rt
}

/// Presses `key` with Shift held, as a Shift+click.
fn shift_press(rt: &mut Runtime<State>, key: &str) {
    let shift = rune::Mods {
        shift: true,
        ..Default::default()
    };
    rt.physical_input(rune::PhysicalInput::Modifiers(shift));
    press(rt, key);
    rt.physical_input(rune::PhysicalInput::Modifiers(Default::default()));
    rt.frame(0.0);
}

/// With multi-model send off, Shift+click picks one model instead of adding it.
fn assert_multi_send_off(rt: &mut Runtime<State>) {
    if rt.find("choose-model-2").is_none() {
        rt.state.open_model_picker(SettingsPicker::Model);
        rt.invalidate();
        rt.frame(0.0);
    }
    shift_press(rt, "choose-model-2");
    assert_eq!(rt.state.multi_model_selections().len(), 1);
}

#[test]
fn multi_model_selection_toggles_persists_across_switch_and_reload_and_respects_availability() {
    let mut rt = setup_multi_model_git_runtime(1280.0);
    let temp_dir = std::env::temp_dir().join(format!(
        "shina-multi-model-persist-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    rt.state.data_dir = temp_dir.clone();
    rt.state.load_global_for_test();
    rt.state.current_mut().draft.focused = true;
    rt.frame(0.0);

    // Open model picker and Shift+click 3 additional models on (4 total), as KSG Code's
    // combobox adds to a multi-model send. Rows have no checkboxes.
    press(&mut rt, "model-effort");
    assert!(rt.presented("model-list").is_some());
    assert!(rt.find("model-multi-toggle-pi-cpa/gpt-5").is_none());
    shift_press(&mut rt, "choose-model-1");
    shift_press(&mut rt, "choose-model-2");
    shift_press(&mut rt, "choose-model-3");
    assert_eq!(rt.state.multi_model_selections().len(), 4);
    assert_eq!(
        rt.state.settings_picker,
        Some(SettingsPicker::Model),
        "Shift+click keeps the picker open"
    );
    assert_eq!(
        rt.state.multi_model_selections()[0].options,
        vec![crate::model_selection::ProviderOptionSelection {
            id: "thinking".into(),
            value: crate::model_selection::OptionValue::Str("high".into()),
        }]
    );
    assert!(rt.state.multi_model_selections()[1].options.is_empty());

    // Shift+click a selected row to take it off.
    shift_press(&mut rt, "choose-model-3");
    assert_eq!(rt.state.multi_model_selections().len(), 3);

    // Switch to a new thread and back; selection persists on the draft.
    let draft_id = rt.state.current().id;
    let project = rt.state.current().project;
    rt.state.new_thread();
    let other_id = rt.state.current().id;
    assert_ne!(other_id, draft_id);
    assert_eq!(rt.state.multi_model_selections().len(), 1);
    let draft_idx = rt
        .state
        .threads
        .iter()
        .position(|t| t.id == draft_id)
        .unwrap();
    rt.state.open_kept_tab(draft_idx);
    assert_eq!(rt.state.multi_model_selections().len(), 3);

    // Persist and reload from temporary data directory.
    rt.state.checkpoint_and_flush().unwrap();
    let mut reloaded = State::new(rt.state.project(project).unwrap().path.clone(), false);
    reloaded.data_dir = temp_dir.clone();
    reloaded.load_snapshot();
    reloaded.catalog.models = rt.state.catalog.models.clone();
    let reloaded_idx = reloaded
        .threads
        .iter()
        .position(|t| t.id == draft_id)
        .unwrap();
    reloaded.open_kept_tab(reloaded_idx);
    reloaded.vcs_branches = rt.state.vcs_branches.clone();
    assert_eq!(reloaded.multi_model_selections().len(), 3);
    assert_eq!(
        reloaded
            .multi_model_selections()
            .iter()
            .map(|s| s.model.as_str())
            .collect::<Vec<_>>(),
        vec!["cpa/claude-sonnet-4-5", "cpa/gpt-5", "cpa/gemini-2-5-pro"]
    );

    // Clicking the model row itself resets to single primary model.
    if rt.find("choose-model-1").is_none() {
        press(&mut rt, "model-effort");
    }
    press(&mut rt, "choose-model-1");
    assert_eq!(rt.state.multi_model_selections().len(), 1);
    assert_eq!(rt.state.current().model_id, "cpa/gpt-5");

    // Unavailable on a thread with messages.
    rt.state.current_mut().messages.push(crate::model::Message {
        id: "m1".into(),
        role: "You",
        text: "Started".into(),
        images: vec![],
        context: None,
    });
    assert!(!rt.state.multiple_models_enabled());
    rt.invalidate();
    rt.frame(1.0);
    rt.frame(1.0);
    assert_multi_send_off(&mut rt);

    // Unavailable on a non-git project.
    rt.state.current_mut().messages.clear();
    let project_path = rt
        .state
        .project(rt.state.current().project)
        .unwrap()
        .path
        .to_string_lossy()
        .into_owned();
    rt.state.vcs_branches.insert(
        ("local".into(), project_path),
        crate::model::VcsBranchCacheEntry {
            is_repo: false,
            loaded: true,
            loading: false,
            error: None,
            refs: Vec::new(),
        },
    );
    assert!(!rt.state.multiple_models_enabled());
    rt.invalidate();
    rt.frame(1.0);
    rt.frame(1.0);
    assert_multi_send_off(&mut rt);

    // Unavailable on a linked machine without vcs-worktrees-v1 capability.
    let project_path = rt
        .state
        .project(rt.state.current().project)
        .unwrap()
        .path
        .to_string_lossy()
        .into_owned();
    rt.state.vcs_branches.insert(
        ("local".into(), project_path),
        crate::model::VcsBranchCacheEntry {
            is_repo: true,
            loaded: true,
            loading: false,
            error: None,
            refs: vec![],
        },
    );
    let (client, _) = crate::client::Client::test_connected(rt.state.data_dir.clone());
    *client.info.lock().unwrap() = json!({"capabilities": ["fresh-context"]});
    rt.state.client = Some(client);
    assert!(!rt.state.multiple_models_enabled());
    rt.invalidate();
    rt.frame(1.0);
    rt.frame(1.0);
    assert_multi_send_off(&mut rt);

    let _ = std::fs::remove_dir_all(&temp_dir);
}
