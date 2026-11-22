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
fn gateway_model_rows_group_family_routes_and_default_medium() {
    // The shape agy 1.5.10 reported in gateway mode on 2026-11-17 (KSG test).
    let advertised: Vec<(String, String)> = [
        ("gemini-4-hydrogen-high", "Gemini 4 Hydrogen"),
        ("gemini-4-hydrogen-xhigh", "Gemini 4 Hydrogen"),
        ("gemini-4-hydrogen", "Gemini 4 Hydrogen"),
        ("gemini-4-hydrogen-low", "Gemini 4 Hydrogen"),
        ("gemini-4-carbon", "Gemini 4 Carbon"),
        ("devin/gemini-4-carbon", "Gemini 4 Carbon"),
        ("gemini-4-argon", "Gemini 4 Argon"),
        ("claude-sonnet-5-5", "Claude Sonnet 5.5"),
        ("claude-opus-5-5", "Claude Opus 5.5"),
        ("gpt-6.2-sol", "GPT 6.2 Sol"),
    ]
    .iter()
    .map(|(v, n)| (v.to_string(), n.to_string()))
    .collect();
    let selections = gateway_model_selections(
        "gemini-4-argon-low, gemini-4-argon-medium, gemini-4-argon-high, \
         gemini-4-hydrogen-low, gemini-4-hydrogen-medium, gemini-4-hydrogen-high, \
         gemini-4-hydrogen-xhigh, \
         gemini-4-carbon-low, gemini-4-carbon-medium, gemini-4-carbon-high, \
         claude-opus-5-5-low, claude-opus-5-5-medium, claude-opus-5-5-high, \
         claude-sonnet-5-5-low, claude-sonnet-5-5-medium, claude-sonnet-5-5-high, \
         gemini-9-unknown-high",
    );
    let rows = gateway_model_rows(&selections, &advertised);
    let summary: Vec<(&str, &str)> = rows
        .iter()
        .map(|row| (row.slug.as_str(), row.name.as_str()))
        .collect();
    assert_eq!(
        summary,
        [
            ("gemini-4-argon", "Gemini 4 Argon"),
            ("gemini-4-hydrogen", "Gemini 4 Hydrogen"),
            ("gemini-4-carbon", "Gemini 4 Carbon"),
            ("claude-opus-5-5", "Claude Opus 5.5"),
            ("claude-sonnet-5-5", "Claude Sonnet 5.5"),
        ]
    );
    let efforts = |slug: &str| {
        rows.iter()
            .find(|row| row.slug == slug)
            .unwrap()
            .efforts
            .iter()
            .map(|e| e.value.as_str())
            .collect::<Vec<_>>()
    };
    let route = |slug: &str| {
        rows.iter()
            .find(|row| row.slug == slug)
            .unwrap()
            .default_route
            .as_str()
    };
    assert_eq!(efforts("gemini-4-argon"), ["low", "medium", "high"]);
    assert_eq!(
        efforts("gemini-4-hydrogen"),
        ["low", "medium", "high", "xhigh"]
    );
    // The agent folds carbon into one family option; the configured tiers stay.
    assert_eq!(efforts("gemini-4-carbon"), ["low", "medium", "high"]);
    assert_eq!(route("gemini-4-carbon"), "gemini-4-carbon-medium");
    assert_eq!(route("gemini-4-hydrogen"), "gemini-4-hydrogen-medium");
    // A single-tier selection keeps its route as the slug.
    let single = gateway_model_rows(&["claude-opus-5-5-high".into()], &advertised);
    assert_eq!(single[0].slug, "claude-opus-5-5-high");
    assert_eq!(single[0].name, "Claude Opus 5.5");
}

#[test]
fn gateway_environment_forwards_gateway_vars_but_not_credentials() {
    let gemini_home = Path::new("/profile");
    let tmp = Path::new("/tmp");
    let to_env = |pairs: &[(&str, &str)]| {
        environment_in(
            gemini_home,
            tmp,
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        )
    };
    fn value<'a>(env: &'a [(String, String)], name: &str) -> Option<&'a str> {
        env.iter()
            .rfind(|(key, _)| key == name)
            .map(|(_, v)| v.as_str())
    }
    // SHINA_* names map onto the AGY_* names agy reads; gateway vars survive.
    let env = to_env(&[
        (
            "SHINA_ANTIGRAVITY_GATEWAY_URL",
            "http://127.0.0.1:8317/antigravity-gateway",
        ),
        ("SHINA_ANTIGRAVITY_API_KEY", "gateway-secret"),
        (
            "SHINA_ANTIGRAVITY_GATEWAY_MODELS",
            "gemini-4-argon, claude-opus-5-5-high",
        ),
        ("SHINA_MCP_TOKEN", "bridge-secret"),
        ("GEMINI_API_KEY", "ambient"),
        ("AGY_LLM_GATEWAY_HEADERS", "x-cpa: 1"),
    ])
    .unwrap();
    assert_eq!(value(&env, "AGY_ACP_ENABLE_GATEWAY_AUTH"), Some("1"));
    assert_eq!(
        value(&env, "AGY_LLM_GATEWAY_URL"),
        Some("http://127.0.0.1:8317/antigravity-gateway")
    );
    assert_eq!(
        value(&env, "AGY_LLM_GATEWAY_API_KEY"),
        Some("gateway-secret")
    );
    assert_eq!(
        value(&env, "AGY_LLM_GATEWAY_MODELS"),
        Some("gemini-4-argon, claude-opus-5-5-high")
    );
    assert_eq!(value(&env, "AGY_LLM_GATEWAY_HEADERS"), Some("x-cpa: 1"));
    assert!(env.iter().all(|(key, _)| !key.starts_with("SHINA_")));
    assert_eq!(value(&env, "GEMINI_API_KEY"), None);
    // AGY_* fallbacks resolve into the canonical LLM gateway names.
    let env = to_env(&[
        ("AGY_GATEWAY_URL", "http://gateway.internal:9000"),
        ("AGY_GATEWAY_API_KEY", "key"),
        ("AGY_LLM_GATEWAY_MODELS", "gemini-4-neon-high"),
    ])
    .unwrap();
    assert_eq!(
        value(&env, "AGY_GATEWAY_URL"),
        Some("http://gateway.internal:9000")
    );
    assert_eq!(
        value(&env, "AGY_LLM_GATEWAY_URL"),
        Some("http://gateway.internal:9000")
    );
    assert_eq!(value(&env, "AGY_LLM_GATEWAY_API_KEY"), Some("key"));
    assert_eq!(
        value(&env, "AGY_LLM_GATEWAY_MODELS"),
        Some("gemini-4-neon-high")
    );
    // Profile isolation still applies.
    assert_eq!(value(&env, "GEMINI_HOME"), Some("/profile"));
    assert_eq!(value(&env, "TMPDIR"), Some("/tmp"));
    assert_eq!(value(&env, "AGY_ACP_FORCE_FILE_STORAGE"), Some("1"));
    // Without a gateway URL nothing gateway-related leaks, and keys stay stripped.
    let env = to_env(&[
        ("AGY_LLM_GATEWAY_HEADERS", "x-cpa: 1"),
        ("AGY_ACP_ENABLE_GATEWAY_AUTH", "1"),
        ("GEMINI_API_KEY", "ambient"),
    ])
    .unwrap();
    assert_eq!(value(&env, "AGY_LLM_GATEWAY_HEADERS"), None);
    assert_eq!(value(&env, "AGY_ACP_ENABLE_GATEWAY_AUTH"), None);
    assert_eq!(value(&env, "GEMINI_API_KEY"), None);
    // A gateway URL that is not http(s) fails before the agent launches.
    assert!(to_env(&[("SHINA_ANTIGRAVITY_GATEWAY_URL", "ftp://gateway.internal")]).is_err());
    assert!(to_env(&[("SHINA_ANTIGRAVITY_GATEWAY_URL", "antigravity-gateway")]).is_err());
    // The profile settings name the resolved auth method.
    assert_eq!(
        profile_settings("gateway"),
        "{\"auth\":{\"type\":\"gateway\"}}\n"
    );
    assert_eq!(
        profile_settings("oauth-personal"),
        "{\"auth\":{\"type\":\"oauth-personal\"}}\n"
    );
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
