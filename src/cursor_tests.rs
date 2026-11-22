//! Unit tests ported from KSG Code's Cursor provider suite
//! (`CursorSdkCatalog.test.ts`, `status.test.ts`, `sdkModel.test.ts` and the
//! `cursor-agent` CLI probe stubs in `ProviderRegistry.test.ts`).
use super::*;
use crate::session::AgentModel;
use std::sync::mpsc;

/// A real file the resolver can find without touching PATH or env vars.
fn fake_cli() -> PathBuf {
    let directory = crate::store::directory().unwrap().join("cursor-tests");
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join(if cfg!(windows) {
        "cursor-agent.exe"
    } else {
        "cursor-agent"
    });
    std::fs::write(&path, b"fixture").unwrap();
    path
}

#[test]
fn executable_resolution_prefers_overrides_then_path_names_then_install_dirs() {
    let candidates = candidates_from(
        [
            Some(OsString::from("C:\\cli\\override.cmd")),
            Some(OsString::from("C:\\cli\\secondary.exe")),
        ],
        Some(PathBuf::from("C:\\Users\\me")),
        Some(PathBuf::from("C:\\Users\\me\\AppData\\Local")),
    );
    assert_eq!(candidates[0], PathBuf::from("C:\\cli\\override.cmd"));
    assert_eq!(candidates[1], PathBuf::from("C:\\cli\\secondary.exe"));
    assert_eq!(candidates[2], PathBuf::from("cursor-agent"));
    assert_eq!(candidates[3], PathBuf::from("agent"));
    if cfg!(windows) {
        assert_eq!(
            candidates[4],
            PathBuf::from("C:\\Users\\me\\AppData\\Local\\cursor-agent\\cursor-agent.exe")
        );
        assert_eq!(
            candidates[5],
            PathBuf::from("C:\\Users\\me\\AppData\\Local\\cursor-agent\\agent.exe")
        );
    } else {
        assert_eq!(
            candidates[4],
            PathBuf::from("C:\\Users\\me/.local/bin/cursor-agent")
        );
        assert_eq!(
            candidates[5],
            PathBuf::from("C:\\Users\\me/.local/bin/agent")
        );
    }
    // Without overrides the PATH names lead; a missing binary reports MISSING.
    let candidates = candidates_from([None, None], None, None);
    assert_eq!(
        candidates[..2],
        [PathBuf::from("cursor-agent"), PathBuf::from("agent")]
    );
    assert_eq!(
        resolve_process(vec![PathBuf::from("definitely-no-cursor-agent-here")]).unwrap_err(),
        MISSING
    );
}

#[test]
fn resolved_process_uses_explicit_env_allowlist() {
    let cli = fake_cli();
    let command = resolve_process(vec![cli.clone()]).unwrap();
    assert_eq!(command.get_program(), cli.as_os_str());
    let envs: std::collections::BTreeMap<_, _> = command
        .get_envs()
        .map(|(name, value)| (name.to_string_lossy().into_owned(), value.is_some()))
        .collect();
    assert_eq!(envs.get("PATH"), Some(&true));
    assert!(!envs.contains_key("SHINA_MCP_URL"));
    assert!(!envs.contains_key("SHINA_MCP_TOKEN"));
    assert!(!envs.contains_key("SHINA_RUNTIME_MODE"));
    assert!(!envs.contains_key("SHINA_PI_BRIDGE_PATH"));
    assert!(!envs.contains_key("SHINA_HONCHO_TOKEN"));
}

#[test]
fn npm_cmd_shims_resolve_through_the_javascript_entry_point() {
    let directory = crate::store::directory().unwrap().join("cursor-tests");
    std::fs::create_dir_all(&directory).unwrap();
    let script = directory.join("cursor-agent.js");
    std::fs::write(&script, "// fixture only").unwrap();
    let shim = directory.join("cursor-agent.cmd");
    std::fs::write(&shim, r#"@"%dp0%\node.exe" "%dp0%\cursor-agent.js" %*"#).unwrap();
    let command = resolve_process(vec![shim]).unwrap();
    assert_eq!(command.get_program().to_string_lossy(), "node");
    assert_eq!(
        command.get_args().next().unwrap().to_string_lossy(),
        script.to_string_lossy()
    );
}

#[test]
fn catalog_models_map_items_to_wire_rows_with_parameter_descriptors() {
    // KSG status.test.ts catalog fixture shape.
    let models = catalog_models(&json!({
        "items": [
            {"id":"gpt-5","displayName":"GPT-5","aliases":["gpt5"," gpt-5-alias ",""],
             "parameters":[
                {"id":"fast","values":[{"value":"false"},{"value":"true"}]},
                {"id":"context","displayName":"Context","values":[{"value":"200k"},{"value":"1m"}]},
                {"id":"effort","displayName":"Effort","values":[{"value":"low"},{"value":"medium","displayName":"Medium"},{"value":"high"}]}
             ],
             "variants":[{"params":[{"id":"effort","value":"high"},{"id":"fast","value":"false"}],
                          "displayName":"Default","isDefault":true}]},
            {"id":"composer-1.5","displayName":"Composer 1.5"},
            {"id":"gpt-5","displayName":"Duplicate"},
            {"id":"  ","displayName":"Blank"},
            {"displayName":"No Id"}
        ]
    }));
    assert_eq!(models.len(), 2);
    let first = &models[0];
    assert_eq!(first["provider"], "cursor");
    assert_eq!(first["id"], "gpt-5");
    assert_eq!(first["name"], "GPT-5");
    assert_eq!(first["aliases"], json!(["gpt5", "gpt-5-alias"]));
    let capabilities = &first["capabilities"]["optionDescriptors"];
    // KSG cursorSdkParameterPriority: effort, context then fast.
    let ids: Vec<_> = capabilities
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(ids, ["effort", "contextWindow", "fastMode"]);
    assert_eq!(capabilities[0]["type"], "select");
    assert_eq!(capabilities[0]["role"], "reasoning");
    assert_eq!(capabilities[0]["options"][2]["id"], "high");
    assert_eq!(capabilities[0]["options"][2]["isDefault"], true);
    assert_eq!(capabilities[1]["options"][0]["label"], "200k");
    // A true/false pair collapses to a Boolean with the variant default.
    assert_eq!(capabilities[2]["type"], "boolean");
    assert_eq!(capabilities[2]["currentValue"], false);
    assert_eq!(models[1]["id"], "composer-1.5");
}

#[test]
fn list_models_reads_cli_lines_and_json_output() {
    let rows = list_models(
        "Available models:\n  - auto\n  - gpt-5\n1. claude-4.5-sonnet\n  opus-4.6 - recommended\n  gpt-5\n3.5-model\n",
    );
    let ids: Vec<_> = rows.iter().map(|m| m["id"].as_str().unwrap()).collect();
    assert_eq!(
        ids,
        [
            "auto",
            "gpt-5",
            "claude-4.5-sonnet",
            "opus-4.6",
            "3.5-model"
        ]
    );
    assert!(rows.iter().all(|m| m["provider"] == "cursor"));
    // JSON output reuses the catalog parser.
    let rows = list_models(r#"{"items":[{"id":"auto","displayName":"Auto"}]}"#);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["name"], "Auto");
    assert!(list_models("").is_empty());
    assert!(list_models("Available models:\n").is_empty());
}

#[test]
fn catalog_rows_round_trip_through_the_native_catalog_model_parse() {
    let row = catalog_models(&json!({
        "items":[{"id":"auto","displayName":"Auto","parameters":[
            {"id":"thinking","values":[{"value":"off"},{"value":"on"}]}]}]
    }))
    .remove(0);
    let model = AgentModel::from_harness_value(&row, "cursor").unwrap();
    assert_eq!(model.provider, "cursor");
    assert_eq!(model.id, "auto");
    assert_eq!(model.name, "Auto");
    assert_eq!(model.thinking_levels, ["off", "on"]);
    assert_eq!(model.capabilities.option_descriptors.len(), 1);
    assert!(
        AgentModel::from_harness_value(&row, "codex").is_none(),
        "rows stay tagged to the cursor provider"
    );
}

#[test]
fn sdk_defaults_come_only_from_the_first_default_variant() {
    let rows = catalog_models(&json!({"items":[
        {"id":"x","displayName":"X","parameters":[
            {"id":"context","values":[{"value":"200k"},{"value":"1m"}]}],
         "variants":[{"isDefault":true,"params":[]},{"isDefault":true,"params":[{"id":"context","value":"1m"}]}]}
    ]}));
    let descriptor = &rows[0]["capabilities"]["optionDescriptors"][0];
    assert_eq!(descriptor["label"], "Contextwindow");
    assert_ne!(descriptor["options"][1]["isDefault"], true);
}

#[test]
fn discovery_records_match_the_native_catalog_contract() {
    let (send, receive) = mpsc::sync_channel(4);
    respond(
        &send,
        "get_available_models",
        Ok(json!({"models":[{"provider":"cursor","id":"auto","name":"Auto"}]})),
    );
    respond(&send, "get_commands", Ok(json!({"commands":commands()})));
    respond(&send, "get_available_models", Err("unauthenticated".into()));
    let models = match receive.recv().unwrap() {
        Event::Record(record) => record,
        _ => panic!("record"),
    };
    assert_eq!(models["type"], "response");
    assert_eq!(models["command"], "get_available_models");
    assert_eq!(models["success"], true);
    let parsed = AgentModel::from_harness_value(&models["data"]["models"][0], "cursor");
    assert_eq!(parsed.unwrap().id, "auto");
    let commands = match receive.recv().unwrap() {
        Event::Record(record) => record,
        _ => panic!("record"),
    };
    assert_eq!(
        commands["data"]["commands"],
        json!([{"name":"compact","description":"Summarize the conversation and reduce context usage","source":"cursor"}])
    );
    let failed = match receive.recv().unwrap() {
        Event::Record(record) => record,
        _ => panic!("record"),
    };
    assert_eq!(failed["success"], false);
    assert_eq!(failed["error"], "unauthenticated");
}

#[test]
fn version_and_url_helpers() {
    assert_eq!(
        cli_version("cursor-agent 2026.05.09"),
        Some("2026.05.09".into())
    );
    assert_eq!(cli_version("agent 1.2.3-4 extra"), Some("1.2.3-4".into()));
    assert_eq!(cli_version("no version"), None);
    assert_eq!(title_case("contextWindow"), "Contextwindow");
    assert_eq!(title_case("fast-mode"), "Fast Mode");
    assert_eq!(title_case("effort"), "Effort");
    assert_eq!(provider_option_id("context"), "contextWindow");
    assert_eq!(provider_option_id("fast"), "fastMode");
    assert_eq!(provider_option_id("effort"), "effort");
    assert!(parameter_priority("effort") < parameter_priority("context"));
    assert!(parameter_priority("fast") < parameter_priority("thinking"));
    assert!(parameter_priority("other") > parameter_priority("thinking"));
}
