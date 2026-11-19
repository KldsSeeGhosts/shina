//! Antigravity (`agy acp`) provider layer. Ported from KSG Code's
//! apps/server/src/provider/{AntigravityProvider.ts, antigravityAuthSupport.ts,
//! AntigravityInstallation.ts, acp/AntigravityAcpSupport.ts,
//! acp/AntigravityClientFiles.ts, acp/AntigravityProtocol.ts}.
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// KSG `ANTIGRAVITY_DEFAULT_MODEL` (contracts/model.ts): alias for the session's current model.
pub const DEFAULT_MODEL: &str = "antigravity-default";
/// KSG `ANTIGRAVITY_AUTH_STDOUT_PREFIX`: the agent prints its OAuth URL after this text.
pub const AUTH_STDOUT_PREFIX: &str = "Open the following link to authenticate the ACP server: ";
/// KSG `ANTIGRAVITY_AUTH_BROWSER_MARKER`: the BROWSER helper echoes the URL on stderr.
pub const AUTH_BROWSER_MARKER: &str = "__T3_ANTIGRAVITY_AUTH_URL__";
/// KSG `ANTIGRAVITY_SIGN_IN_REQUIRED_MESSAGE`.
pub const SIGN_IN_REQUIRED: &str = "Sign in to Antigravity: open the sign-in link when it appears, or run `agy acp --login` in a terminal with the Shina profile below, then retry.";
const MAX_AUTH_URL: usize = 16_384;
const MAX_HELPER_LINE: usize = AUTH_BROWSER_MARKER.len() + MAX_AUTH_URL + 2;
/// KSG `CLIENT_FILE_MAX_BYTES`.
const CLIENT_FILE_MAX: u64 = 8 * 1024 * 1024;

/// KSG `ANTIGRAVITY_GATEWAY_EFFORT_OPTION_ID`: the per-row route picker the
/// adapter owns in gateway mode; it never reaches the agent as a config option.
pub const GATEWAY_EFFORT_OPTION_ID: &str = "reasoningEffort";
/// KSG `EFFORTS` in AntigravityGatewayModels.ts, in descriptor order.
const GATEWAY_EFFORTS: &[(&str, &str)] = &[
    ("minimal", "Minimal"),
    ("low", "Low"),
    ("medium", "Medium"),
    ("high", "High"),
    ("xhigh", "Extra High"),
    ("max", "Max"),
];

/// KSG `parseAntigravityVersion`: first dotted triple, anything else is unknown.
pub fn parse_version(version: Option<&str>) -> Option<[u64; 3]> {
    let version = version?.trim();
    let mut parts = version.split('.');
    Some([
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    ])
}
/// KSG `compareVersions`.
fn cmp(a: [u64; 3], b: [u64; 3]) -> std::cmp::Ordering {
    a.cmp(&b)
}
/// KSG `antigravityWritesRunWorkflowSkill`: 1.4.2 through 1.4.5 write run-workflow/SKILL.md.
pub fn writes_run_workflow_skill(version: Option<&str>) -> bool {
    parse_version(version).is_some_and(|v| cmp(v, [1, 4, 2]).is_ge() && cmp(v, [1, 4, 5]).is_le())
}
/// KSG `isQualifiedAntigravityRelease`: only 1.4.5 keeps native reads and linked video.
pub fn is_qualified_release(version: Option<&str>) -> bool {
    version.is_some_and(|v| v.trim() == "1.4.5")
}
/// KSG `OPEN_MODEL_ACCESS_RELEASE`/`antigravityClientInfo`: 1.5.1+ gets Shina's name and
/// version; older runtimes only offer all models to allowlisted clients such as Zed.
pub fn client_info(version: Option<&str>) -> Value {
    if parse_version(version).is_some_and(|v| cmp(v, [1, 5, 1]).is_ge()) {
        json!({"name":"Shina","title":"Shina","version":env!("CARGO_PKG_VERSION")})
    } else {
        json!({"name":"zed","version":"0.0.0"})
    }
}
/// KSG `antigravityAppendsSystemPrompt`: 1.4.5 or the advertised `antigravity.systemPrompt.append`.
pub fn appends_system_prompt(initialize: &Value) -> bool {
    is_qualified_release(initialize["agentInfo"]["version"].as_str())
        || initialize["agentCapabilities"]["_meta"]["antigravity"]["systemPrompt"]["append"] == true
}
/// KSG `antigravitySessionMeta`: `_meta.antigravity` for session/new, load and resume.
pub fn session_meta(initialize: &Value, append: Option<&str>) -> Option<Value> {
    let mut antigravity = json!({});
    if writes_run_workflow_skill(initialize["agentInfo"]["version"].as_str()) {
        antigravity["disabledTools"] = json!(["run_workflow"]);
    }
    if let Some(append) = append.filter(|_| appends_system_prompt(initialize)) {
        antigravity["systemPrompt"] = json!({"append":append});
    }
    (antigravity.as_object()?.is_empty() == false).then(|| json!({"antigravity":antigravity}))
}
/// KSG `antigravityPermissionMode`: Shina access modes to Antigravity session modes.
pub fn permission_mode(runtime_mode: &str) -> &'static str {
    match runtime_mode {
        "full-access" => "yolo",
        "auto-accept-edits" => "auto_edit",
        _ => "default",
    }
}
/// KSG `antigravityCliSessionMode`: the agy CLI rejects yolo/auto_edit; default applies the
/// live runtime policy each turn.
pub fn cli_session_mode(mode_id: &str) -> &str {
    match mode_id {
        "yolo" | "auto_edit" => "default",
        other => other,
    }
}
/// KSG `antigravityRuntimeModeForSessionMode`.
pub fn runtime_mode_for_session_mode(mode_id: &str) -> Option<&'static str> {
    Some(match mode_id {
        "yolo" => "full-access",
        "auto_edit" => "auto-accept-edits",
        "default" => "approval-required",
        _ => return None,
    })
}

/// KSG `selectConfig`: the first matching config option of type select.
pub fn select_config<'a>(
    options: &'a [Value],
    matches: impl Fn(&Value) -> bool,
) -> Option<&'a Value> {
    options
        .iter()
        .find(|o| matches(o))
        .filter(|o| o["type"] == "select")
}
/// KSG `selectValues`: flat select values, expanding option groups.
pub fn select_values(option: &Value) -> Vec<&Value> {
    option["options"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|entry| {
            if entry["value"].is_string() {
                vec![entry]
            } else {
                entry["options"].as_array().into_iter().flatten().collect()
            }
        })
        .collect()
}
/// KSG `antigravityModelConfig`.
pub fn model_config(options: &[Value]) -> Option<&Value> {
    select_config(options, |o| o["id"] == "model" || o["category"] == "model")
}
/// KSG `antigravityModelOptions`.
pub fn model_options(options: &[Value]) -> Vec<&Value> {
    model_config(options).map(select_values).unwrap_or_default()
}
/// KSG `antigravityThinkingConfig`: a separate thinking select, any 1.4+ family session.
pub fn thinking_config(options: &[Value]) -> Option<&Value> {
    select_config(options, |o| {
        o["category"] == "thought_level" || o["id"] == "thinking_level"
    })
}
/// KSG `AcpSessionRuntime.setMode`: the session's mode is its `category: "mode"`
/// config option when one exists.
pub fn mode_config(options: &[Value]) -> Option<&Value> {
    select_config(options, |o| o["category"] == "mode")
}
/// KSG `isModeChoice`: mode-category config picks share the session-mode ceiling.
pub fn is_mode_choice(options: &[Value], option_id: &str) -> bool {
    option_id == crate::descriptors::ACP_SESSION_MODE_OPTION_ID
        || options.iter().any(|o| {
            o["id"] == option_id
                && matches!(o["category"].as_str(), Some("mode" | "collaboration_mode"))
        })
}
/// KSG `currentAntigravityModel`: family plus thinking level recombines into the
/// 1.3-style combined id the picker keeps.
pub fn current_model(options: &[Value]) -> Option<String> {
    let model = model_config(options)?["currentValue"].as_str()?;
    match thinking_config(options).and_then(|c| c["currentValue"].as_str()) {
        Some(level) if !level.is_empty() => Some(format!("{model}-{level}")),
        _ => Some(model.to_owned()),
    }
}
/// KSG `splitAntigravityModel`: `gemini-3.8-flash-high` into family `gemini-3.8-flash`
/// plus level `high`, longest family prefix first.
pub fn split_model(options: &[Value], model_id: &str) -> Option<(String, Option<String>)> {
    let families: Vec<String> = model_config(options)
        .map(|c| {
            select_values(c)
                .into_iter()
                .filter_map(|e| e["value"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    if families.iter().any(|f| f == model_id) {
        return Some((model_id.into(), None));
    }
    thinking_config(options)?;
    let mut family: Vec<_> = families
        .into_iter()
        .filter(|f| model_id.starts_with(&format!("{f}-")))
        .collect();
    family.sort_by_key(|f| std::cmp::Reverse(f.len()));
    let family = family.into_iter().next()?;
    Some((family.clone(), Some(model_id[family.len() + 1..].into())))
}
/// KSG `legacyAntigravityFamily`: a combined 1.3 id the agent still accepts.
pub fn legacy_family(options: &[Value], model: &str) -> Option<String> {
    let mut family: Vec<_> = model_options(options)
        .into_iter()
        .filter_map(|o| o["value"].as_str().map(str::to_owned))
        .filter(|f| model.starts_with(&format!("{f}-")))
        .collect();
    family.sort_by_key(|f| std::cmp::Reverse(f.len()));
    family.into_iter().next()
}
/// KSG `resolveAntigravityModel`: saved selection wins; an advertised or legacy default
/// beats the runtime's current model.
pub fn resolve_model(
    options: &[Value],
    model: Option<&str>,
    default: Option<&str>,
) -> Option<String> {
    if let Some(model) = model.filter(|m| *m != DEFAULT_MODEL) {
        return Some(model.into());
    }
    let values: Vec<&str> = model_options(options)
        .into_iter()
        .filter_map(|o| o["value"].as_str())
        .collect();
    if let Some(default) = default
        && (values.contains(&default) || legacy_family(options, default).is_some())
    {
        return Some(default.into());
    }
    current_model(options)
}

/// KSG `routeFamily`: the family part of a `family-effort` route id.
fn route_family(id: &str) -> &str {
    match id.rsplit_once('-') {
        Some((family, suffix)) if GATEWAY_EFFORTS.iter().any(|(e, _)| suffix == *e) => family,
        _ => id,
    }
}
/// KSG `ROUTE_SUFFIX.test`: whether an id ends in a known `-effort` route suffix.
fn has_route_suffix(id: &str) -> bool {
    id.rsplit_once('-')
        .is_some_and(|(_, suffix)| GATEWAY_EFFORTS.iter().any(|(e, _)| suffix == *e))
}
/// KSG `matchesRoute`: an exact route, or a family selection matching the route's family.
fn matches_route(selection: &str, id: &str) -> bool {
    selection == id || (!has_route_suffix(selection) && selection == route_family(id))
}
/// KSG `antigravityGatewayModelSelections`: comma list, `(effort)` settings tails
/// folded into route ids, deduplicated, blanks dropped.
pub fn gateway_model_selections(configured: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut selections = Vec::new();
    for id in configured.split(',') {
        let id = id.trim();
        let normalized = match id.rfind('(') {
            Some(open)
                if id.ends_with(')')
                    && GATEWAY_EFFORTS
                        .iter()
                        .any(|(e, _)| id[open + 1..id.len() - 1].eq_ignore_ascii_case(e)) =>
            {
                format!(
                    "{}-{}",
                    &id[..open],
                    id[open + 1..id.len() - 1].to_lowercase()
                )
            }
            _ => id.to_owned(),
        };
        if !normalized.is_empty() && seen.insert(normalized.clone()) {
            selections.push(normalized);
        }
    }
    selections
}
/// KSG `antigravityHasConfiguredGatewayModel`: exact, family or route-family match.
pub fn has_configured_gateway_model(selections: &[String], model: &str) -> bool {
    selections
        .iter()
        .any(|s| matches_route(s, model) || route_family(s) == model)
}

/// KSG `AntigravityGatewayModelOption` rows: a family of selectable routes.
pub struct GatewayRow {
    pub slug: String,
    pub name: String,
    pub routes: Vec<(String, String)>,
    pub efforts: Vec<GatewayEffort>,
    pub default_route: String,
}
pub struct GatewayEffort {
    pub value: String,
    pub label: &'static str,
    pub route: String,
    pub is_default: bool,
}

fn gateway_insert(
    grouped: &mut Vec<(String, Vec<(String, String)>)>,
    index: &mut std::collections::HashMap<String, usize>,
    family: &str,
    value: &str,
    name: &str,
) {
    let position = *index.entry(family.to_owned()).or_insert_with(|| {
        grouped.push((family.to_owned(), vec![]));
        grouped.len() - 1
    });
    let entries = &mut grouped[position].1;
    if let Some(existing) = entries.iter_mut().find(|(v, _)| v == value) {
        *existing = (value.to_owned(), name.to_owned());
    } else {
        entries.push((value.to_owned(), name.to_owned()));
    }
}
/// KSG `LABEL_SUFFIX`: a display name's trailing ` (effort)` is dropped.
fn strip_label_suffix(name: &str) -> &str {
    if let Some(open) = name.rfind(" (") {
        let inner = &name[open + 2..];
        if inner.ends_with(')') {
            let label = inner[..inner.len() - 1].to_ascii_lowercase();
            if matches!(
                label.as_str(),
                "minimal" | "low" | "medium" | "high" | "xhigh" | "extra high" | "max"
            ) {
                return &name[..open];
            }
        }
    }
    name
}
/// KSG `antigravityGatewayModelRows`: group configured routes by family. A configured
/// tier the agent folds into the family option stays, named after the family.
pub fn gateway_model_rows(selections: &[String], models: &[(String, String)]) -> Vec<GatewayRow> {
    let mut grouped: Vec<(String, Vec<(String, String)>)> = vec![];
    let mut index = std::collections::HashMap::new();
    for selection in selections {
        let mut matched = false;
        for (value, name) in models {
            if !matches_route(selection, value) {
                continue;
            }
            matched = true;
            let family = route_family(value).to_owned();
            gateway_insert(&mut grouped, &mut index, &family, value, name);
        }
        if matched || !has_route_suffix(selection) {
            continue;
        }
        // The agent folds tier routes into one family option and can omit tiers it
        // does not know. The gateway still serves a configured tier of a family the
        // agent advertises, so keep it rather than hide a working level.
        let family = route_family(selection).to_owned();
        if let Some((_, name)) = models
            .iter()
            .find(|(v, _)| v == &family || route_family(v) == family)
        {
            gateway_insert(&mut grouped, &mut index, &family, selection, name);
        }
    }
    grouped
        .into_iter()
        .map(|(family, entries)| {
            let first = entries.first().cloned().unwrap_or_default();
            let mut efforts: Vec<GatewayEffort> = GATEWAY_EFFORTS
                .iter()
                .filter_map(|(effort, label)| {
                    entries
                        .iter()
                        .find(|(v, _)| v == &format!("{family}-{effort}"))
                        .map(|(v, _)| GatewayEffort {
                            value: (*effort).to_owned(),
                            label,
                            route: v.clone(),
                            is_default: false,
                        })
                })
                .collect();
            let default = efforts
                .iter()
                .position(|e| e.value == "medium")
                .unwrap_or(0);
            if let Some(effort) = efforts.get_mut(default) {
                effort.is_default = true;
            }
            let family_selected = selections.iter().any(|s| s == &family)
                || selections
                    .iter()
                    .filter(|s| route_family(s) == family)
                    .count()
                    > 1;
            let name = {
                let trimmed = strip_label_suffix(first.1.trim());
                if trimmed.is_empty() {
                    family.clone()
                } else {
                    trimmed.to_owned()
                }
            };
            GatewayRow {
                slug: if family_selected {
                    family
                } else {
                    first.0.clone()
                },
                name,
                routes: entries,
                default_route: efforts
                    .get(default)
                    .map(|e| e.route.clone())
                    .unwrap_or_else(|| first.0.clone()),
                efforts,
            }
        })
        .collect()
}

/// KSG `isAntigravitySignInRequiredError`: -32000, or -32603 "No browser is available here",
/// or Shina's own sign-in-required transport detail.
pub fn is_sign_in_required_error(code: i64, message: &str) -> bool {
    code == -32000 || (code == -32603 && message.starts_with("No browser is available here"))
}
/// KSG `parseAntigravityAuthorizationUrl`: only Google's loopback OAuth start is shown.
pub fn parse_authorization_url(url: &str) -> Result<String, String> {
    let invalid = || "Antigravity returned an invalid Google sign-in URL.".to_owned();
    if url.len() > MAX_AUTH_URL || url.chars().any(char::is_whitespace) {
        return Err(invalid());
    }
    let url = url.trim_end_matches(['\r', '\n']);
    let parsed = url::Url::parse(url).map_err(|_| invalid())?;
    let get_all = |key: &str| parsed.query_pairs().filter(|(k, _)| k == key).count();
    let state = parsed
        .query_pairs()
        .find(|(k, _)| k == "state")
        .map(|(_, v)| v.into_owned());
    let redirect = parsed
        .query_pairs()
        .find(|(k, _)| k == "redirect_uri")
        .map(|(_, v)| v.into_owned());
    let valid = parsed.scheme() == "https"
        && parsed.domain() == Some("accounts.google.com")
        && parsed.path() == "/o/oauth2/v2/auth"
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.fragment().is_none()
        && get_all("state") == 1
        && get_all("redirect_uri") == 1
        && get_all("response_type") == 1
        && parsed
            .query_pairs()
            .any(|(k, v)| k == "response_type" && v == "code")
        && state.as_deref().is_some_and(|s| {
            !s.is_empty() && s.len() <= 512 && !s.chars().any(char::is_whitespace)
        })
        && redirect.as_deref().is_some_and(|r| {
            let Some(port) = r
                .strip_prefix("http://127.0.0.1:")
                .and_then(|r| r.strip_suffix('/'))
            else {
                return false;
            };
            !port.is_empty()
                && port.bytes().all(|c| c.is_ascii_digit())
                && port.parse::<u32>().is_ok_and(|port| port >= 1024)
        });
    valid.then(|| url.to_owned()).ok_or_else(invalid)
}
/// KSG `makeAntigravityStderrHandler` line handling: marker JSON or the plain prefix.
pub fn stderr_auth_url(line: &str) -> Option<String> {
    let message = line.strip_suffix('\r').unwrap_or(line);
    if message.len() > MAX_HELPER_LINE {
        return None;
    }
    let url = message
        .strip_prefix(AUTH_STDOUT_PREFIX)
        .map(str::to_owned)
        .or_else(|| {
            message
                .strip_prefix(AUTH_BROWSER_MARKER)
                .and_then(|rest| serde_json::from_str::<String>(rest).ok())
        })?;
    parse_authorization_url(&url).ok()
}
/// KSG `makeAntigravityStdoutTransform` line handling.
pub fn stdout_auth_url(line: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(line)
        .ok()?
        .trim_end_matches(['\r', '\n']);
    parse_authorization_url(text.strip_prefix(AUTH_STDOUT_PREFIX)?).ok()
}

/// KSG `maxStdoutLineBytes` bounds a single protocol line.
const MAX_STDOUT_LINE: u64 = 16 * 1024 * 1024;
pub enum AgentLine {
    Record(Value),
    AuthUrl(String),
    Eof,
}
/// KSG `makeAntigravityStdoutTransform`: split stdout into lines, lift the OAuth
/// prefix line out of the protocol stream, bound unfinished lines.
pub fn read_agent_record(reader: &mut impl std::io::BufRead) -> Result<AgentLine, String> {
    use std::io::{BufRead, Read};
    let mut bytes = Vec::new();
    let count = Read::by_ref(reader)
        .take(MAX_STDOUT_LINE + 1)
        .read_until(b'\n', &mut bytes)
        .map_err(|e| format!("Could not read ACP output: {e}"))?;
    if count == 0 {
        return Ok(AgentLine::Eof);
    }
    if count as u64 > MAX_STDOUT_LINE {
        return Err("Antigravity sent a protocol line that is too large.".into());
    }
    if let Some(url) = stdout_auth_url(&bytes) {
        return Ok(AgentLine::AuthUrl(url));
    }
    serde_json::from_slice(&bytes)
        .map(AgentLine::Record)
        .map_err(|e| format!("ACP sent an invalid RPC record: {e}"))
}

/// KSG `resolveClientFilePath` + `readAntigravityClientTextFile`: serve `fs/read_text_file`
/// only for real files under the session workspace, 8 MiB, UTF-8 without NUL, ACP 1-indexed
/// line/limit slicing.
pub fn read_text_file(request: &Value, roots: &[&Path]) -> Result<Value, (i64, String)> {
    let path = request["path"].as_str().unwrap_or("");
    let outside = || {
        (
            -32602,
            format!("Path '{path}' is outside the session workspace."),
        )
    };
    let resolved = Path::new(path);
    let mut candidate = resolved.to_path_buf();
    let mut suffix = vec![];
    let real = loop {
        if let Ok(real) = candidate.canonicalize() {
            break suffix.iter().rev().fold(real, |p, s| p.join(s));
        }
        if candidate.is_symlink() || candidate.exists() {
            return Err(outside());
        }
        let Some(parent) = candidate.parent() else {
            return Err(outside());
        };
        if parent == candidate {
            return Err(outside());
        }
        suffix.push(candidate.file_name().unwrap_or_default().to_owned());
        candidate = parent.to_path_buf();
    };
    let inside = roots.iter().any(|root| {
        let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        real == root || real.starts_with(&root)
    });
    if !inside {
        return Err(outside());
    }
    let meta =
        std::fs::metadata(&real).map_err(|_| (-32002, format!("File '{path}' not found.")))?;
    if !meta.is_file() || meta.len() > CLIENT_FILE_MAX {
        return Err((
            -32602,
            format!("File '{path}' is not a readable text file under {CLIENT_FILE_MAX} bytes."),
        ));
    }
    let bytes = std::fs::read(&real).map_err(|_| (-32603, format!("Could not read '{path}'.")))?;
    let text = String::from_utf8(bytes)
        .map_err(|_| (-32602, format!("File '{path}' is not a UTF-8 text file.")))?;
    if text.contains('\0') {
        return Err((-32602, format!("File '{path}' contains binary data.")));
    }
    let line = request["line"].as_u64();
    let limit = request["limit"].as_u64();
    if line.is_none() && limit.is_none() {
        return Ok(json!({"content":text}));
    }
    let lines: Vec<&str> = text.split('\n').collect();
    let start = line.unwrap_or(1).saturating_sub(1) as usize;
    let end = limit.map_or(lines.len(), |l| (start + l as usize).min(lines.len()));
    Ok(json!({"content":lines[start.min(lines.len())..end.max(start).min(lines.len())].join("\n")}))
}

/// KSG `sanitizeToolValue`/`sanitizeAntigravityToolPayload`: bound tool payload size and
/// drop image blobs and duplicate formatted output.
pub fn sanitize_tool_value(value: &Value, budget: &mut (i64, i64), depth: usize) -> Option<Value> {
    const LIMIT: i64 = 8_000;
    if depth > 12 || budget.0 <= 0 {
        return None;
    }
    budget.0 -= 1;
    match value {
        Value::String(text) => {
            if text.to_ascii_lowercase().starts_with("data:image/") || budget.1 <= 0 {
                return None;
            }
            let cut = text
                .encode_utf16()
                .take(LIMIT.min(budget.1) as usize)
                .count();
            let text: String = text.chars().take(cut).collect();
            budget.1 -= text.len() as i64;
            Some(json!(text))
        }
        Value::Array(list) => Some(
            list.iter()
                .filter_map(|v| sanitize_tool_value(v, budget, depth + 1))
                .collect(),
        ),
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (key, entry) in map {
                if budget.0 <= 0 {
                    break;
                }
                let skip = map.get("type") == Some(&json!("image"))
                    && (key == "data" || key == "blob")
                    || key == "blob"
                        && map
                            .get("mimeType")
                            .and_then(Value::as_str)
                            .is_some_and(|m| m.starts_with("image/"))
                    || (key == "formatted_output" || key == "formattedOutput")
                        && (entry == map.get("combinedOutput").unwrap_or(&Value::Null)
                            || entry == map.get("combined_output").unwrap_or(&Value::Null));
                if !skip && let Some(v) = sanitize_tool_value(entry, budget, depth + 1) {
                    out.insert(key.clone(), v);
                }
            }
            Some(Value::Object(out))
        }
        _ => Some(value.clone()),
    }
}
/// KSG `normalizeAntigravitySessionUpdate`: bound and strip oversized tool payloads.
pub fn normalize_session_update(notification: &Value) -> Value {
    let update = &notification["update"];
    if !matches!(
        update["sessionUpdate"].as_str(),
        Some("tool_call" | "tool_call_update")
    ) {
        return notification.clone();
    }
    let mut update = update.clone();
    for key in ["title", "rawInput", "rawOutput", "_meta"] {
        let mut budget = (512, 32_000);
        if let Some(v) = sanitize_tool_value(&update[key], &mut budget, 0) {
            update[key] = v;
        } else {
            update[key] = Value::Null;
        }
    }
    let mut budget = (512, 32_000);
    if let Some(content) = update["content"].as_array() {
        update["content"] = json!(
            content
                .iter()
                .filter_map(|entry| sanitize_tool_value(entry, &mut budget, 0))
                .collect::<Vec<_>>()
        );
    }
    let mut out = notification.clone();
    out["update"] = update;
    out
}

/// KSG `resolveAntigravityInstanceDirectories`, single built-in instance: private
/// GEMINI_HOME plus a runtime temp under Shina's data dir.
pub fn directories() -> Result<(PathBuf, PathBuf), String> {
    let state = crate::store::directory().ok_or("Could not locate Shina's data directory.")?;
    Ok((
        state.join("providers").join("antigravity"),
        state.join("antigravity-tmp"),
    ))
}
/// KSG `AntigravityAuthConfig` for the gateway method, resolved from the environment.
/// `SHINA_ANTIGRAVITY_*` names win; the `AGY_*` names `agy acp` reads are fallbacks.
pub struct Gateway {
    pub url: String,
    pub api_key: Option<String>,
    pub models: String,
}
/// KSG `antigravityAuthConfigIssue` for `gateway`: the URL must be http(s).
fn is_http_url(value: &str) -> bool {
    url::Url::parse(value).is_ok_and(|url| {
        matches!(url.scheme(), "http" | "https") && url.host_str().is_some_and(|h| !h.is_empty())
    })
}
fn gateway_from(get: impl Fn(&str) -> Option<String>) -> Result<Option<Gateway>, String> {
    let value = |name: &str| {
        get(name)
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
    };
    let Some(url) = value("SHINA_ANTIGRAVITY_GATEWAY_URL")
        .or_else(|| value("AGY_LLM_GATEWAY_URL"))
        .or_else(|| value("AGY_GATEWAY_URL"))
    else {
        return Ok(None);
    };
    if !is_http_url(&url) {
        return Err("The Antigravity gateway URL must be an http or https URL.".into());
    }
    Ok(Some(Gateway {
        url,
        api_key: value("SHINA_ANTIGRAVITY_API_KEY")
            .or_else(|| value("AGY_LLM_GATEWAY_API_KEY"))
            .or_else(|| value("AGY_GATEWAY_API_KEY")),
        models: value("SHINA_ANTIGRAVITY_GATEWAY_MODELS")
            .or_else(|| value("AGY_LLM_GATEWAY_MODELS"))
            .unwrap_or_default(),
    }))
}
/// The gateway configured for the next `agy` launch, or `Err` for a set but invalid URL.
pub fn gateway() -> Result<Option<Gateway>, String> {
    gateway_from(|name| std::env::var(name).ok())
}
/// KSG `authMethod` resolution from the environment: `gateway` when a gateway URL is set.
fn auth_method() -> Result<&'static str, String> {
    Ok(if gateway()?.is_some() {
        "gateway"
    } else {
        "oauth-personal"
    })
}
/// KSG `antigravityProfileSettings`: names the auth method so a logout clears only it.
fn profile_settings(auth_method: &str) -> String {
    format!("{{\"auth\":{{\"type\":\"{auth_method}\"}}}}\n")
}
/// KSG `prepareAntigravityProfile`: private profile dirs (0700) and settings.json.
pub fn prepare_profile() -> Result<(PathBuf, PathBuf, PathBuf), String> {
    let (gemini_home, tmp) = directories()?;
    let acp = gemini_home.join("antigravity-acp");
    for dir in [&gemini_home, &acp, &tmp] {
        std::fs::create_dir_all(dir)
            .map_err(|_| "The Antigravity profile directory could not be created.".to_owned())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        }
    }
    std::fs::write(acp.join("settings.json"), profile_settings(auth_method()?))
        .map_err(|_| "The Antigravity profile settings could not be written.".to_owned())?;
    Ok((gemini_home, acp, tmp))
}
/// KSG `removedEnvironmentKeys`: credentials and harness state never leak into the profile.
const REMOVED_ENV: &[&str] = &[
    "GEMINI_API_KEY",
    "GOOGLE_API_KEY",
    "GOOGLE_APPLICATION_CREDENTIALS",
    "GOOGLE_CLOUD_PROJECT",
    "GOOGLE_CLOUD_LOCATION",
    "GOOGLE_CLOUD_QUOTA_PROJECT",
    "GOOGLE_GENAI_USE_VERTEXAI",
    "GCLOUD_PROJECT",
    "CLOUDSDK_CORE_PROJECT",
    "AGY_ACP_CCPA_PROJECT",
    "AGY_ACP_ENABLE_OAUTH",
    "AGY_ACP_ENABLE_GATEWAY_AUTH",
    "AGY_LLM_GATEWAY_URL",
    "AGY_LLM_GATEWAY_API_KEY",
    "AGY_LLM_GATEWAY_HEADERS",
    "AGY_LLM_GATEWAY_MODELS",
    "AGY_GATEWAY_URL",
    "AGY_GATEWAY_API_KEY",
    "AGY_GATEWAY_HEADERS",
    "GEMINI_HOME",
    "AGY_ACP_FORCE_FILE_STORAGE",
    "ANTIGRAVITY_HARNESS_PATH",
    "BROWSER",
    "PYTHONUNBUFFERED",
    "ELECTRON_RUN_AS_NODE",
    "CHROME_CRASHPAD_PIPE_NAME",
];
/// Gateway/CPA names `agy acp` may read. They survive `env_clear` only once a gateway
/// is resolved, so a stray `AGY_*` value can never half-enable the gateway method.
const GATEWAY_ENV: &[&str] = &[
    "AGY_ACP_ENABLE_GATEWAY_AUTH",
    "AGY_LLM_GATEWAY_URL",
    "AGY_LLM_GATEWAY_API_KEY",
    "AGY_LLM_GATEWAY_HEADERS",
    "AGY_LLM_GATEWAY_MODELS",
    "AGY_GATEWAY_URL",
    "AGY_GATEWAY_API_KEY",
    "AGY_GATEWAY_HEADERS",
];
/// KSG `antigravityEnvironment`: filtered base env plus profile isolation. BROWSER runs
/// Shina's own helper so the agent's browser open lands as a stderr marker instead.
pub fn environment(gemini_home: &Path, tmp: &Path) -> Result<Vec<(String, String)>, String> {
    environment_in(gemini_home, tmp, std::env::vars().collect())
}
fn environment_in(
    gemini_home: &Path,
    tmp: &Path,
    base: Vec<(String, String)>,
) -> Result<Vec<(String, String)>, String> {
    let helper = std::env::current_exe().map_err(|e| e.to_string())?;
    let quote = |v: &str| format!("'{}'", v.replace('\'', "'\"'\"'"));
    let browser = [
        helper.to_string_lossy().into_owned(),
        "--antigravity-browser-helper".into(),
        "%s".into(),
    ]
    .iter()
    .map(|a| quote(a))
    .collect::<Vec<_>>()
    .join(" ");
    if browser.contains(if cfg!(windows) { ';' } else { ':' })
        || helper
            .to_string_lossy()
            .chars()
            .any(|c| matches!(c, '\r' | '\n' | '\0'))
        || helper.to_string_lossy().contains("%s")
    {
        return Err(
            "The Shina runtime path cannot be used to suppress Antigravity browser launches."
                .into(),
        );
    }
    let lookup = |name: &str| {
        base.iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.clone())
    };
    let mut env: Vec<(String, String)> = base
        .iter()
        .filter(|(key, _)| {
            !REMOVED_ENV.contains(&key.to_uppercase().as_str()) && !key.starts_with("SHINA_")
        })
        .cloned()
        .collect();
    if let Some(gateway) = gateway_from(&lookup)? {
        // KSG authEnvironment for the gateway method. Parent-exported gateway/CPA
        // names pass through verbatim; the resolved set then overrides, so the
        // SHINA_ names and fallbacks win wherever they disagree.
        for (key, value) in &base {
            if GATEWAY_ENV.contains(&key.to_uppercase().as_str()) && !value.is_empty() {
                env.push((key.clone(), value.clone()));
            }
        }
        // The runtime keeps its gateway method behind this rollout flag.
        env.extend([
            ("AGY_ACP_ENABLE_GATEWAY_AUTH".into(), "1".into()),
            ("AGY_LLM_GATEWAY_URL".into(), gateway.url),
        ]);
        if let Some(api_key) = gateway.api_key {
            env.push(("AGY_LLM_GATEWAY_API_KEY".into(), api_key));
        }
        if !gateway.models.is_empty() {
            env.push(("AGY_LLM_GATEWAY_MODELS".into(), gateway.models));
        }
    }
    env.extend([
        (
            "GEMINI_HOME".into(),
            gemini_home.to_string_lossy().into_owned(),
        ),
        ("AGY_ACP_FORCE_FILE_STORAGE".into(), "1".into()),
        ("BROWSER".into(), browser),
        ("PYTHONUNBUFFERED".into(), "1".into()),
        ("TMPDIR".into(), tmp.to_string_lossy().into_owned()),
    ]);
    // KSG `authMethod: "gateway"`: providers.json or SHINA_ANTIGRAVITY_* can
    // point `agy` at a CLIPROXYAPI-style gateway. REMOVED_ENV already kept the
    // AGY_* names out of the inherited list, so these only ever append.
    let providers = crate::provider_config::Registry::read();
    if let Some(error) = providers.error {
        return Err(error);
    }
    if let Some(gateway) = providers.gateway("antigravity") {
        env.extend(
            gateway
                .antigravity_environment()
                .into_iter()
                .map(|(name, value)| (name, value.to_string_lossy().into_owned())),
        );
    }
    Ok(env)
}
/// KSG `buildAntigravityAcpSpawnInput` for the agy CLI (harnessPath null): `agy acp`
/// with its private `--gemini-dir`/`--app-data-dir` and the isolated environment.
pub fn command(workspace: &Path) -> Result<std::process::Command, String> {
    let (executable, _) = crate::harness::executable("agy")?;
    let (gemini_home, _, tmp) = prepare_profile()?;
    let mut command = std::process::Command::new(executable);
    command
        .args(["acp", "--gemini-dir"])
        .arg(&gemini_home)
        .args(["--app-data-dir", "antigravity-acp"])
        .current_dir(workspace)
        .env_clear()
        .envs(environment(&gemini_home, &tmp)?);
    Ok(command)
}
/// KSG `fromExternal` PATH probe. Tests stay hermetic: the host's installed
/// `agy` must not change picker snapshots, so the probe is off under cfg(test)
/// and tests inject the provider explicitly instead.
pub fn cli_available() -> bool {
    #[cfg(test)]
    return false;
    #[cfg(not(test))]
    crate::harness::executable("agy").is_ok()
}
/// Version for `client_info`/file-read gating; unknown stays None like KSG's path installs.
pub fn cli_version() -> Option<String> {
    let (executable, _) = crate::harness::executable("agy").ok()?;
    let out = std::process::Command::new(executable)
        .arg("--version")
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let found = text
        .split_whitespace()
        .find(|t| parse_version(Some(t)).is_some())?;
    Some(
        found
            .trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '.')
            .to_owned(),
    )
}
/// The BROWSER helper: print marker + JSON URL on stderr, exit 0 (KSG `browserHelperSource`).
pub fn browser_helper(url: &str) {
    eprintln!(
        "{AUTH_BROWSER_MARKER}{}",
        serde_json::to_string(url).unwrap_or_default()
    );
}
/// Session start: private profile, isolated `agy acp` command, shared ACP runtime.
pub fn start(
    start: crate::harness::Start,
    wake: rune::Waker,
) -> Result<crate::backend::Pi, String> {
    let agent = crate::harness::Agent {
        id: "antigravity".into(),
        name: "Antigravity".into(),
        command: "agy".into(),
        args: vec![],
        env: vec![],
    };
    let command = command(&start.workspace)?;
    crate::acp::start_command(agent, start, wake, command)
}
/// Live catalog probe via `agy acp`, matching KSG's `AntigravityProvider` session discovery.
pub fn discover(
    workspace: std::path::PathBuf,
    wake: rune::Waker,
) -> Result<crate::backend::Pi, String> {
    // ACP publishes models and commands once the temporary session is ready.
    start(
        crate::harness::Start::discovery("antigravity", workspace),
        wake,
    )
}
/// KSG `buildAntigravityModelsFromSession`. With a configured gateway model list it
/// exposes only the configured families, each effort riding one `reasoningEffort`
/// descriptor instead of the agent's thinking select. Otherwise model ids stay
/// combined `family-level`.
pub fn session_models(
    setup: &Value,
    thinking_levels: &std::collections::HashMap<String, Option<Value>>,
    gateway_models: &str,
) -> (Vec<Value>, Option<String>) {
    let options = setup["configOptions"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let combined = thinking_config(&options).is_some() && setup["models"].is_object();
    let current = if combined {
        current_model(&options).or_else(|| {
            setup["models"]["currentModelId"]
                .as_str()
                .map(str::to_owned)
        })
    } else {
        model_config(&options)
            .and_then(|c| c["currentValue"].as_str().map(str::to_owned))
            .or_else(|| {
                setup["models"]["currentModelId"]
                    .as_str()
                    .map(str::to_owned)
            })
    };
    let entries: Vec<(String, String)> = if !combined && let Some(config) = model_config(&options) {
        select_values(config)
            .into_iter()
            .filter_map(|e| {
                Some((
                    e["value"].as_str()?.into(),
                    e["name"].as_str().unwrap_or("").into(),
                ))
            })
            .collect()
    } else {
        setup["models"]["availableModels"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| {
                Some((
                    m["modelId"].as_str()?.into(),
                    m["name"].as_str().unwrap_or("").into(),
                ))
            })
            .collect()
    };
    let selections = gateway_model_selections(gateway_models);
    if !selections.is_empty() {
        let all: Vec<(String, String)> = entries
            .iter()
            .cloned()
            .chain(model_options(&options).into_iter().filter_map(|e| {
                Some((
                    e["value"].as_str()?.to_owned(),
                    e["name"].as_str().unwrap_or("").to_owned(),
                ))
            }))
            .collect();
        let rows = gateway_model_rows(&selections, &all);
        let preferred = rows.iter().find(|row| {
            row.routes
                .iter()
                .any(|(v, _)| current.as_deref() == Some(v.as_str()))
        });
        let default_slug = preferred
            .or_else(|| rows.first())
            .map(|row| row.slug.clone());
        let models = rows
            .iter()
            .map(|row| {
                json!({"slug":row.slug,"name":row.name,
                    "isDefault":Some(row.slug.as_str())==default_slug.as_deref(),
                    "aliases":(Some(row.slug.as_str())==default_slug.as_deref()).then(||vec![DEFAULT_MODEL]),
                    "efforts":row.efforts.iter().map(|e| json!({"id":e.value,"label":e.label,"isDefault":e.is_default})).collect::<Vec<_>>()})
            })
            .collect();
        return (models, default_slug);
    }
    let current_thinking = thinking_config(&options);
    let mut seen = std::collections::HashSet::new();
    let mut names = std::collections::HashSet::new();
    let mut models = vec![];
    for (value, name) in entries {
        if value.trim().is_empty() || !seen.insert(value.clone()) {
            continue;
        }
        let base = if name.trim().is_empty() {
            value.clone()
        } else {
            name
        };
        let name = if names.contains(&base) {
            format!("{base} ({value})")
        } else {
            base
        };
        names.insert(name.clone());
        models.push(json!({"slug":value,"name":name,
            "isDefault":current.as_deref()==Some(value.as_str()),
            "aliases":(current.as_deref()==Some(value.as_str())).then(||vec![DEFAULT_MODEL]),
            "thinking":thinking_levels.get(&value).cloned().flatten().or_else(||current_thinking.cloned().filter(|_|current.as_deref()==Some(value.as_str())))}));
    }
    (models, current)
}
/// KSG `observeThinkingLevels`: remember each family's thinking select once selected.
pub fn observe_thinking_levels(
    options: &[Value],
    thinking_levels: &mut std::collections::HashMap<String, Option<Value>>,
) {
    if let Some(model) = options
        .iter()
        .find(|o| (o["id"] == "model" || o["category"] == "model") && o["type"] == "select")
        .and_then(|o| o["currentValue"].as_str())
    {
        thinking_levels.insert(model.into(), thinking_config(options).cloned());
    }
}
/// Manifest `antigravity` section names/status for picker display.
pub fn manifest() -> &'static Value {
    static MANIFEST: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    MANIFEST.get_or_init(|| {
        serde_json::from_str::<Value>(include_str!("../assets/model-manifest.json"))
            .unwrap_or_default()["providers"]["antigravity"]
            .clone()
    })
}
/// KSG `catalogServerModel`/`applyModelCatalog` join: display name, legacy badge, alias.
pub fn apply_manifest(models: &mut Vec<Value>) {
    let entries = manifest()["models"].as_array().cloned().unwrap_or_default();
    for model in models.iter_mut() {
        let slug = model["slug"].as_str().unwrap_or("");
        let Some(entry) = entries.iter().find(|e| e["slug"] == slug) else {
            continue;
        };
        if let Some(name) = entry["name"].as_str() {
            model["name"] = json!(name);
        }
        if entry["status"] == "legacy" {
            model["isLegacy"] = json!(true);
        }
    }
    if !models.iter().any(|m| m["isDefault"] == true)
        && let Some(default) = manifest()["defaults"]["chat"].as_str()
        && let Some(model) = models.iter_mut().find(|m| m["slug"] == default)
    {
        model["isDefault"] = json!(true);
        model["aliases"] = json!([DEFAULT_MODEL]);
    }
}
/// Fallback Antigravity catalog from `assets/model-manifest.json` before a live session reports models.
pub fn manifest_models() -> Vec<crate::session::AgentModel> {
    let manifest = manifest();
    let default_slug = manifest["defaults"]["chat"].as_str();
    let Some(entries) = manifest["models"].as_array() else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|entry| {
            let slug = entry["slug"].as_str()?.trim();
            if slug.is_empty() {
                return None;
            }
            let name = entry["name"]
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(slug)
                .to_owned();
            let is_default = default_slug == Some(slug);
            Some(crate::session::AgentModel {
                provider: "antigravity".into(),
                id: slug.to_owned(),
                name,
                aliases: if is_default {
                    vec![DEFAULT_MODEL.to_owned()]
                } else {
                    vec![]
                },
                is_default,
                is_legacy: entry["status"] == "legacy",
                ..Default::default()
            })
        })
        .collect()
}
/// Open the captured sign-in URL on user click only (platform opener, detached).
pub fn open_browser(url: &str) -> Result<(), String> {
    parse_authorization_url(url)?;
    let mut command = if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        c.arg(url);
        c
    } else if cfg!(windows) {
        let mut c = std::process::Command::new("cmd");
        c.args(["/c", "start", "", url]);
        c
    } else {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(url);
        c
    };
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
#[path = "antigravity_tests.rs"]
mod tests;
