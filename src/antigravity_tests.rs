//! Unit tests ported from KSG Code's AntigravityAcpSupport.test.ts,
//! AntigravityProvider.test.ts and antigravityAuthSupport.test.ts.
use super::*;
use serde_json::json;

#[test]
fn version_parsing_and_qualified_release_rules() {
    assert_eq!(parse_version(Some("1.4.5")), Some([1, 4, 5]));
    assert_eq!(parse_version(Some("  1.5.10 ")), Some([1, 5, 10]));
    assert_eq!(parse_version(Some("agy 1.5.1")), None);
    assert_eq!(parse_version(Some("1.4")), None);
    assert_eq!(parse_version(None), None);
    assert!(is_qualified_release(Some("1.4.5")));
    assert!(!is_qualified_release(Some("1.4.6")));
    assert!(!is_qualified_release(None));
    for (version, writes) in [
        ("1.4.1", false),
        ("1.4.2", true),
        ("1.4.4", true),
        ("1.4.5", true),
        ("1.4.6", false),
        ("1.5.10", false),
    ] {
        assert_eq!(
            writes_run_workflow_skill(Some(version)),
            writes,
            "{version}"
        );
    }
    // 1.4.5 is qualified AND writes the skill; the meta carries both rules.
    let meta = session_meta(
        &json!({"agentInfo":{"version":"1.4.5"}}),
        Some("instructions"),
    )
    .unwrap();
    assert_eq!(
        meta["antigravity"]["disabledTools"],
        json!(["run_workflow"])
    );
    assert_eq!(
        meta["antigravity"]["systemPrompt"]["append"],
        "instructions"
    );
}

#[test]
fn client_info_by_version() {
    assert_eq!(client_info(Some("1.5.1"))["name"], "Shina");
    assert_eq!(client_info(Some("1.5.10"))["title"], "Shina");
    assert_eq!(
        client_info(Some("1.4.5")),
        json!({"name":"zed","version":"0.0.0"})
    );
    assert_eq!(client_info(None), json!({"name":"zed","version":"0.0.0"}));
}

#[test]
fn permission_modes_round_trip() {
    assert_eq!(permission_mode("full-access"), "yolo");
    assert_eq!(permission_mode("auto-accept-edits"), "auto_edit");
    assert_eq!(permission_mode("auto"), "default");
    assert_eq!(permission_mode("approval-required"), "default");
    assert_eq!(cli_session_mode("yolo"), "default");
    assert_eq!(cli_session_mode("auto_edit"), "default");
    assert_eq!(cli_session_mode("plan"), "plan");
    assert_eq!(runtime_mode_for_session_mode("yolo"), Some("full-access"));
    assert_eq!(
        runtime_mode_for_session_mode("auto_edit"),
        Some("auto-accept-edits")
    );
    assert_eq!(
        runtime_mode_for_session_mode("default"),
        Some("approval-required")
    );
    assert_eq!(runtime_mode_for_session_mode("other"), None);
}

fn config_options() -> Vec<Value> {
    vec![
        json!({"id":"model","name":"Model","category":"model","type":"select",
            "currentValue":"gemini-3.8-flash",
            "options":[{"value":"gemini-3.8-flash","name":"Gemini 3.8 Flash"},
                {"value":"gemini-4-carbon","name":"Gemini 4 Carbon"}]}),
        json!({"id":"thinking_level","name":"Thinking","category":"thought_level",
            "type":"select","currentValue":"high",
            "options":[{"value":"low","name":"Low"},{"value":"medium","name":"Medium"},
                {"value":"high","name":"High"}]}),
    ]
}

#[test]
fn model_thinking_split_and_combine() {
    let options = config_options();
    assert_eq!(
        current_model(&options).as_deref(),
        Some("gemini-3.8-flash-high")
    );
    assert_eq!(
        split_model(&options, "gemini-3.8-flash-high"),
        Some(("gemini-3.8-flash".into(), Some("high".into())))
    );
    assert_eq!(
        split_model(&options, "gemini-4-carbon"),
        Some(("gemini-4-carbon".into(), None))
    );
    assert_eq!(split_model(&options, "unrelated-thing"), None);
    // Legacy 1.3 combined ids still resolve to their family.
    assert_eq!(
        legacy_family(&options, "gemini-3.8-flash-low"),
        Some("gemini-3.8-flash".to_string())
    );
    assert_eq!(
        resolve_model(&options, Some("gemini-3.8-flash-high"), None).as_deref(),
        Some("gemini-3.8-flash-high")
    );
    assert_eq!(
        resolve_model(&options, Some(DEFAULT_MODEL), Some("gemini-4-carbon")).as_deref(),
        Some("gemini-4-carbon")
    );
    assert_eq!(
        resolve_model(&options, None, None).as_deref(),
        Some("gemini-3.8-flash-high")
    );
    // A thinking config without options still splits; without one nothing splits.
    let family_only = vec![config_options()[0].clone()];
    assert_eq!(split_model(&family_only, "gemini-3.8-flash-high"), None);
    assert_eq!(
        current_model(&family_only).as_deref(),
        Some("gemini-3.8-flash")
    );
}

#[test]
fn session_models_combined_ids_and_manifest() {
    let options = config_options();
    let setup = json!({"configOptions":options,
        "models":{"currentModelId":"gemini-3.8-flash",
            "availableModels":[{"modelId":"gemini-3.8-flash-high","name":"Flash"},
                {"modelId":"gemini-3.8-flash-medium","name":"Flash"},
                {"modelId":"gemini-4-carbon","name":"Carbon"}]}});
    let mut thinking_levels = std::collections::HashMap::new();
    observe_thinking_levels(&options, &mut thinking_levels);
    assert!(thinking_levels["gemini-3.8-flash"].is_some());
    let (mut models, current) = session_models(&setup, &thinking_levels);
    apply_manifest(&mut models);
    assert_eq!(current.as_deref(), Some("gemini-3.8-flash-high"));
    let default = models.iter().find(|m| m["isDefault"] == true).unwrap();
    assert_eq!(default["slug"], "gemini-3.8-flash-high");
    assert_eq!(default["aliases"], json!([DEFAULT_MODEL]));
    assert_eq!(models.len(), 3);
    // Manifest renames to the shared catalog names and marks legacy rows.
    assert_eq!(models[0]["name"], "Gemini 3.8 Flash (High)");
    assert_eq!(models[2]["name"], "Gemini 4 Carbon");
}

#[test]
fn auth_url_extraction_validates_google_loopback() {
    let url = "https://accounts.google.com/o/oauth2/v2/auth?client_id=x&redirect_uri=http%3A%2F%2F127.0.0.1%3A8085%2F&response_type=code&state=abc";
    assert_eq!(parse_authorization_url(url).unwrap(), url);
    assert!(
        stderr_auth_url(&format!(
            "{AUTH_BROWSER_MARKER}{}",
            serde_json::to_string(url).unwrap()
        ))
        .is_some()
    );
    assert!(stdout_auth_url(format!("{AUTH_STDOUT_PREFIX}{url}\n").as_bytes()).is_some());
    for bad in [
        "https://evil.example.com/o/oauth2/v2/auth?redirect_uri=http%3A%2F%2F127.0.0.1%3A8085%2F&response_type=code&state=a",
        "https://accounts.google.com/o/oauth2/v2/auth?response_type=code&state=a",
        "https://accounts.google.com/o/oauth2/v2/auth?redirect_uri=http%3A%2F%2F127.0.0.1%3A80%2F&response_type=code&state=a",
        "https://accounts.google.com/o/oauth2/v2/auth?redirect_uri=http%3A%2F%2F127.0.0.1%3A8085%2F&response_type=code&state=a b",
        "log in to continue at https://accounts.google.com",
    ] {
        assert!(parse_authorization_url(bad).is_err(), "{bad}");
        assert!(
            stdout_auth_url(format!("{bad}\n").as_bytes()).is_none(),
            "{bad}"
        );
    }
    // Similar login text is not a URL; oversized lines are dropped.
    assert!(stderr_auth_url("please sign in soon").is_none());
    assert!(stderr_auth_url(&"x".repeat(MAX_HELPER_LINE + 1)).is_none());
    assert!(is_sign_in_required_error(-32000, "Authentication required"));
    assert!(is_sign_in_required_error(
        -32603,
        "No browser is available here, do x"
    ));
    assert!(!is_sign_in_required_error(-32602, "Invalid params"));
}

#[test]
fn read_text_file_bounds_roots_and_lines() {
    let dir = crate::store::directory().unwrap();
    let workspace = dir.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let file = workspace.join("note.txt");
    std::fs::write(&file, "one\ntwo\nthree").unwrap();
    let path = file.to_string_lossy().into_owned();
    let read = |params: Value| read_text_file(&params, &[workspace.as_path()]);
    assert_eq!(
        read(json!({"path":path})).unwrap()["content"],
        "one\ntwo\nthree"
    );
    assert_eq!(
        read(json!({"path":path,"line":2,"limit":1})).unwrap()["content"],
        "two"
    );
    assert!(read(json!({"path":"/etc/passwd"})).is_err());
    assert!(read(json!({"path":"/definitely/missing/file"})).is_err());
}

#[test]
fn session_update_normalization_sanitizes_tool_payloads() {
    let notification = json!({"sessionId":"s","update":{
        "sessionUpdate":"tool_call","toolCallId":"t","title":"Run",
        "content":[{"type":"content","content":{"type":"text","text":"hi"}}],
        "_meta":{"image":{"type":"image","data":"data:image/png;base64,AAA"}},
        "rawInput":{"note":"x".repeat(40_000)}}});
    let normalized = normalize_session_update(&notification);
    let update = &normalized["update"];
    assert!(
        update["_meta"]["image"]["data"].is_null()
            || update["_meta"]["image"].get("data").is_none()
    );
    let note = update["rawInput"]["note"].as_str().unwrap();
    assert!(note.len() < 40_000);
    // Non-tool updates pass through untouched.
    let other = json!({"sessionId":"s","update":{"sessionUpdate":"plan"}});
    assert_eq!(normalize_session_update(&other), other);
}

#[test]
fn manifest_models_populate_catalog_before_live_session() {
    let models = manifest_models();
    assert!(!models.is_empty());
    let default = models
        .iter()
        .find(|m| m.id == "gemini-3.8-flash-high")
        .expect("default antigravity model from manifest");
    assert_eq!(default.provider, "antigravity");
    assert_eq!(default.name, "Gemini 3.8 Flash (High)");
    assert!(default.is_default);
    assert!(!default.is_legacy);
    assert!(default.aliases.iter().any(|a| a == DEFAULT_MODEL));
    let legacy = models
        .iter()
        .find(|m| m.id == "gemini-3.7-flash-high")
        .expect("legacy antigravity model from manifest");
    assert!(legacy.is_legacy);
}
