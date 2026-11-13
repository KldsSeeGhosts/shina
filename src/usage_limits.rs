//! Shared usageLimits contract: {windows:[{id,name,usedPercent,resetsAt}]}
//! plus KSG Code's `UsageLimitsReport` types and pure builders.
//! resetsAt is Unix seconds in the legacy `windows` block, ISO-8601 in `UsageLimitsReport`.

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const USAGE_LIMITS_COMMAND_NAME: &str = "usage-limits";
pub const USAGE_LIMITS_COMMAND_DESCRIPTION: &str = "Show this provider's usage limits";

const MINUTE_MS: i64 = 60_000;
const HOUR_MS: i64 = 60 * MINUTE_MS;
const DAY_MS: i64 = 24 * HOUR_MS;
const SESSION_MINS: u64 = 5 * 60;
const WEEK_MINS: u64 = 7 * 24 * 60;
const MONTH_MINS: u64 = 30 * 24 * 60;

pub fn authentication_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    [
        "authentication",
        "unauthorized",
        "unauthorised",
        "invalid api key",
        "not logged in",
        "not authenticated",
        "login required",
        "please log in",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

pub fn codex(snapshot: &Value, previous: &Value) -> Value {
    // KSG keeps model-specific pools out of the main Codex account allowance.
    if snapshot["limitId"].as_str().is_some_and(|id| id != "codex") {
        return previous.clone();
    }
    let mut windows = previous["windows"].as_array().cloned().unwrap_or_default();
    let limit = snapshot["limitId"].as_str().unwrap_or("codex");
    for (key, fallback) in [("primary", "Primary"), ("secondary", "Secondary")] {
        let window = &snapshot[key];
        let Some(percent) = window["usedPercent"].as_f64().filter(|n| n.is_finite()) else {
            continue;
        };
        let id = format!("{limit}:{key}");
        let duration = window["windowDurationMins"].as_u64();
        let name = match duration {
            Some(n) if n >= 1440 && n % 1440 == 0 => format!("{} day", n / 1440),
            Some(n) if n >= 60 && n % 60 == 0 => format!("{} hour", n / 60),
            Some(n) => format!("{n} minute"),
            None => fallback.into(),
        };
        let name = snapshot["limitName"]
            .as_str()
            .map_or(name.clone(), |limit| format!("{limit} {name}"));
        let normalized = json!({"id":id,"name":name,"usedPercent":percent.clamp(0.0,100.0),
            "resetsAt":window["resetsAt"]});
        if let Some(old) = windows.iter_mut().find(|window| window["id"] == id) {
            *old = normalized;
        } else {
            windows.push(normalized);
        }
    }
    if let Some(remaining) = snapshot["individualLimit"]["remainingPercent"]
        .as_f64()
        .filter(|n| n.is_finite())
    {
        let normalized = json!({"id":"codex:individual","name":"Monthly credit limit",
            "usedPercent":(100.0-remaining).clamp(0.0,100.0),
            "resetsAt":snapshot["individualLimit"]["resetsAt"]});
        if let Some(old) = windows
            .iter_mut()
            .find(|window| window["id"] == "codex:individual")
        {
            *old = normalized;
        } else {
            windows.push(normalized);
        }
    } else if snapshot.get("individualLimit") == Some(&Value::Null) {
        windows.retain(|window| window["id"] != "codex:individual");
    }
    json!({"windows":windows})
}

pub fn claude(info: &Value, previous: &Value) -> Value {
    let mut windows = previous["windows"].as_array().cloned().unwrap_or_default();
    let id = info["rateLimitType"].as_str().unwrap_or("usage");
    if let Some(percent) = info["utilization"].as_f64().filter(|n| n.is_finite()) {
        let normalized = json!({"id":id,"name":id.replace('_'," "),
            "usedPercent":(percent * 100.0).clamp(0.0,100.0),"resetsAt":info["resetsAt"]});
        if let Some(old) = windows.iter_mut().find(|window| window["id"] == id) {
            *old = normalized;
        } else {
            windows.push(normalized);
        }
    }
    json!({"windows":windows})
}

/// Ported from KSG Code's `isUsageLimitsCommand` (`packages/shared/src/usageLimits.ts`).
pub fn is_usage_limits_command(prompt: &str) -> bool {
    prompt.trim().eq_ignore_ascii_case("/usage-limits")
}

/// Only Codex and Claude have provider usage-limit reports in Shina, matching KSG Code's
/// `hasProviderUsageLimits` for the harnesses Shina implements.
pub fn has_usage_limits(harness: &str) -> bool {
    matches!(harness, "codex" | "claude")
}

/// Ported verbatim from KSG Code's `redactedPlaceholder`
/// (`apps/web/src/components/settings/RedactedSensitiveText.tsx`).
pub fn redacted_placeholder(value: &str) -> String {
    const ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
    let mut state: u32 = 0x811c_9dc5;
    for unit in value.encode_utf16() {
        state ^= u32::from(unit);
        state = state.wrapping_mul(0x0100_0193);
    }

    let mut next_char = || -> char {
        state = (state ^ (state >> 13)).wrapping_mul(0x85eb_ca6b);
        state = (state ^ (state >> 16)).wrapping_mul(0xc2b2_ae35);
        let signed = state as i32;
        let abs = i64::from(signed).abs() as usize;
        char::from(ALPHABET[abs % ALPHABET.len()])
    };

    value
        .chars()
        .map(|ch| {
            if matches!(ch, '@' | '.' | '-' | '_') {
                ch
            } else {
                next_char()
            }
        })
        .collect()
}

pub fn conceal_account_identifier(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(redacted_placeholder(trimmed))
    }
}

/// Conceal any `email` field in an account JSON blob before storing or logging.
pub fn conceal_account_json(account: &mut Value) {
    if let Some(obj) = account.as_object_mut()
        && obj.get("emailMasked").and_then(Value::as_bool) != Some(true)
        && let Some(email) = obj.get("email").and_then(Value::as_str).map(str::to_owned)
    {
        if let Some(masked) = conceal_account_identifier(&email) {
            obj.insert("email".into(), Value::String(masked));
            obj.insert("emailMasked".into(), Value::Bool(true));
        } else {
            obj.remove("email");
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UsageWindowKind {
    Session,
    Weekly,
    Monthly,
    Other,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerProviderUsageWindow {
    pub id: String,
    pub kind: UsageWindowKind,
    pub label: String,
    pub used_percent: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_duration_mins: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerProviderResetCreditOption {
    pub credit_id: String,
    pub label: String,
    pub description: String,
    pub available_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerProviderResetCredits {
    pub available_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_expires_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_credit_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<ServerProviderResetCreditOption>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalUsage {
    pub label: String,
    pub url: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UsageUnavailableReason {
    Unsupported,
    ProbeFailed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageUnavailable {
    pub reason: UsageUnavailableReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerProviderUsageLimits {
    pub checked_at: String,
    pub windows: Vec<ServerProviderUsageWindow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reset_credits: Option<ServerProviderResetCredits>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_usage: Option<ExternalUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unavailable: Option<UsageUnavailable>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ProviderConsumeResetCreditInput {
    Instance {
        #[serde(rename = "instanceId")]
        instance_id: String,
        #[serde(rename = "creditId", default, skip_serializing_if = "Option::is_none")]
        credit_id: Option<String>,
    },
    Source {
        #[serde(rename = "sourceId")]
        source_id: String,
        #[serde(rename = "accountId")]
        account_id: String,
        #[serde(rename = "creditId")]
        credit_id: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageLimitsAccount {
    pub id: String,
    pub driver: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reset_credit_input: Option<ProviderConsumeResetCreditInput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent_color: Option<String>,
    pub limits: ServerProviderUsageLimits,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageLimitsReport {
    pub created_at: String,
    pub accounts: Vec<UsageLimitsAccount>,
    pub notices: Vec<String>,
}

pub fn iso_from_epoch_ms(ms: i64) -> String {
    DateTime::<Utc>::from_timestamp_millis(ms)
        .unwrap_or(DateTime::<Utc>::UNIX_EPOCH)
        .to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub fn iso_from_epoch_seconds(value: f64) -> Option<String> {
    if !value.is_finite() || value <= 0.0 {
        return None;
    }
    let ms = (value * 1000.0).round() as i64;
    DateTime::<Utc>::from_timestamp_millis(ms)
        .map(|dt| dt.to_rfc3339_opts(SecondsFormat::Millis, true))
}

pub fn parse_iso_ms(value: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

fn clamp_percent(value: f64) -> f64 {
    if !value.is_finite() {
        0.0
    } else {
        value.clamp(0.0, 100.0)
    }
}

#[allow(dead_code)]
pub fn remaining_percent(window: &ServerProviderUsageWindow) -> i64 {
    (100.0 - clamp_percent(window.used_percent)).round() as i64
}

/// Ported from KSG Code's `formatDuration` (`packages/shared/src/usageLimits.ts`).
pub fn format_duration(ms: i64) -> String {
    let remaining = ms.max(0);
    let days = remaining / DAY_MS;
    let hours = (remaining % DAY_MS) / HOUR_MS;
    let minutes = (remaining % HOUR_MS) / MINUTE_MS;
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else {
        format!("{minutes}m")
    }
}

/// Ported from KSG Code's `formatResetsIn` (`packages/shared/src/usageLimits.ts`).
pub fn format_resets_in(window: &ServerProviderUsageWindow, now_ms: i64) -> Option<String> {
    let resets_at = window.resets_at.as_deref().and_then(parse_iso_ms)?;
    if resets_at <= now_ms {
        return Some("resets now".into());
    }
    if resets_at - now_ms < MINUTE_MS {
        return Some("Resets momentarily".into());
    }
    Some(format!("resets in {}", format_duration(resets_at - now_ms)))
}

/// Ported from KSG Code's `limitsNotice` (`packages/shared/src/usageLimits.ts`).
pub fn limits_notice(limits: &ServerProviderUsageLimits) -> Option<String> {
    if let Some(unavailable) = &limits.unavailable {
        return Some(match unavailable.reason {
            UsageUnavailableReason::Unsupported => unavailable
                .message
                .clone()
                .unwrap_or_else(|| "This account has no subscription limits.".into()),
            UsageUnavailableReason::ProbeFailed => unavailable
                .message
                .clone()
                .unwrap_or_else(|| "Could not read limits.".into()),
        });
    }
    if limits.windows.is_empty() {
        Some("No limits reported.".into())
    } else {
        None
    }
}

/// Ported from KSG Code's `ResetCreditChoice` and `resetCreditChoices` (`packages/shared/src/usageLimits.ts`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResetCreditChoice {
    pub key: String,
    pub summary: String,
    pub action_label: String,
    pub confirm_description: String,
    pub available_count: u64,
    pub input: ProviderConsumeResetCreditInput,
}

pub fn reset_credit_choices(
    credits: &ServerProviderResetCredits,
    input: &ProviderConsumeResetCreditInput,
    now_ms: i64,
    compact: bool,
) -> Vec<ResetCreditChoice> {
    let expires_in = |at: Option<&str>| -> Option<String> {
        let ms = at.and_then(parse_iso_ms)?;
        Some(format_duration(ms - now_ms))
    };
    if let Some(options) = &credits.options
        && let ProviderConsumeResetCreditInput::Instance { instance_id, .. } = input
    {
        return options
            .iter()
            .map(|option| {
                let expires = expires_in(option.expires_at.as_deref());
                ResetCreditChoice {
                    key: option.credit_id.clone(),
                    summary: format!(
                        "{} · {} left{}",
                        option.label,
                        option.available_count,
                        expires
                            .as_deref()
                            .map_or(String::new(), |e| format!(" · expires in {e}"))
                    ),
                    action_label: "Use".into(),
                    confirm_description: format!(
                        "{}: {} This cannot be undone.",
                        option.label, option.description
                    ),
                    available_count: option.available_count,
                    input: ProviderConsumeResetCreditInput::Instance {
                        instance_id: instance_id.clone(),
                        credit_id: Some(option.credit_id.clone()),
                    },
                }
            })
            .collect();
    }
    let count = credits.available_count;
    let expires = expires_in(credits.next_expires_at.as_deref());
    let summary = if count == 0 {
        "No reset credits banked".into()
    } else if compact {
        format!(
            "{count} banked{}",
            expires
                .as_deref()
                .map_or(String::new(), |e| format!(" · expires in {e}"))
        )
    } else {
        format!(
            "{count} {} banked{}",
            if count == 1 {
                "reset credit"
            } else {
                "reset credits"
            },
            expires
                .as_deref()
                .map_or(String::new(), |e| format!(" · next expires in {e}"))
        )
    };
    vec![ResetCreditChoice {
        key: "next".into(),
        summary,
        action_label: "Use reset".into(),
        confirm_description: "This redeems one credit on your account and clears the current rate-limit windows. It cannot be undone.".into(),
        available_count: count,
        input: input.clone(),
    }]
}

fn driver_display_name(driver: &str) -> &str {
    match driver {
        "codex" => "Codex",
        "claude" | "claudeAgent" => "Claude",
        "pi" => "Pi",
        other => other,
    }
}

/// Ported from KSG Code's `accountLabel` and `AccountSummary` (`ComposerUsageLimits.tsx`).
pub fn account_label(account: &UsageLimitsAccount) -> String {
    let Some(instance_id) = account.instance_id.as_deref() else {
        return account.label.clone();
    };
    let driver = driver_display_name(&account.driver);
    let instance = account
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| (instance_id != account.driver).then_some(instance_id))
        .unwrap_or("");
    if !instance.is_empty() && !instance.eq_ignore_ascii_case(driver) {
        format!("{driver} · {instance}")
    } else {
        driver.to_owned()
    }
}

pub fn account_summary(account: &UsageLimitsAccount) -> String {
    let label = account_label(account);
    match account
        .plan
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(plan) => format!("{label} · {plan}"),
        None => label,
    }
}

/// Ported from KSG Code's `codexPlanLabel` (`apps/server/src/provider/CodexProvider.ts`).
pub fn codex_plan_label(plan_type: Option<&str>) -> Option<&'static str> {
    match plan_type? {
        "free" => Some("ChatGPT Free Subscription"),
        "go" => Some("ChatGPT Go Subscription"),
        "plus" => Some("ChatGPT Plus Subscription"),
        "pro" => Some("ChatGPT Pro 200 Subscription"),
        "prolite" => Some("ChatGPT Pro 100 Subscription"),
        "promax" => Some("ChatGPT Pro 500 Subscription"),
        "self_serve_business_prolite" => Some("ChatGPT Business Premium Subscription"),
        "team" | "self_serve_business_usage_based" | "business" => {
            Some("ChatGPT Business Subscription")
        }
        "ent26" | "enterprise_cbp_automation" | "enterprise_cbp_usage_based" | "enterprise" => {
            Some("ChatGPT Enterprise Subscription")
        }
        "edu" | "edu_plus" | "edu_pro" => Some("ChatGPT Edu Subscription"),
        "unknown" => Some("ChatGPT Subscription"),
        _ => None,
    }
}

pub fn codex_account_auth_label(
    account: Option<&Value>,
    usage_plan_type: Option<&str>,
) -> Option<String> {
    let account = account?;
    match account["type"].as_str()? {
        "apiKey" => Some("OpenAI API Key".into()),
        "amazonBedrock" => Some("Amazon Bedrock".into()),
        "chatgpt" => {
            let usage_label = if usage_plan_type == Some("unknown") {
                None
            } else {
                codex_plan_label(usage_plan_type)
            };
            usage_label
                .or_else(|| codex_plan_label(account["planType"].as_str()))
                .map(str::to_owned)
        }
        _ => None,
    }
}

fn kind_for_duration(mins: u64) -> UsageWindowKind {
    if mins >= MONTH_MINS {
        UsageWindowKind::Monthly
    } else if mins >= WEEK_MINS {
        UsageWindowKind::Weekly
    } else {
        UsageWindowKind::Session
    }
}

fn label_for_kind(kind: UsageWindowKind) -> &'static str {
    match kind {
        UsageWindowKind::Session => "Session",
        UsageWindowKind::Weekly => "Weekly",
        UsageWindowKind::Monthly => "Monthly",
        UsageWindowKind::Other => "Usage",
    }
}

fn codex_rate_limits_to_windows(snapshot: &Value) -> Vec<ServerProviderUsageWindow> {
    if let Some(limit_id) = snapshot["limitId"].as_str()
        && limit_id != "codex"
    {
        return Vec::new();
    }
    let is_monthly_plan = matches!(snapshot["planType"].as_str(), Some("free" | "go"));
    let positions = [
        (
            "primary",
            if is_monthly_plan {
                MONTH_MINS
            } else {
                SESSION_MINS
            },
        ),
        ("secondary", WEEK_MINS),
    ];
    let mut windows = Vec::new();
    for (id, fallback_mins) in positions {
        let window = &snapshot[id];
        let Some(used_percent) = window["usedPercent"].as_f64().filter(|n| n.is_finite()) else {
            continue;
        };
        let window_duration_mins = window["windowDurationMins"]
            .as_u64()
            .unwrap_or(fallback_mins);
        let kind = kind_for_duration(window_duration_mins);
        let resets_at = window["resetsAt"].as_f64().and_then(iso_from_epoch_seconds);
        windows.push(ServerProviderUsageWindow {
            id: id.into(),
            kind,
            label: label_for_kind(kind).into(),
            used_percent: clamp_percent(used_percent),
            resets_at,
            window_duration_mins: Some(window_duration_mins),
            estimated: None,
        });
    }
    let individual = &snapshot["individualLimit"];
    if let Some(remaining) = individual["remainingPercent"]
        .as_f64()
        .filter(|n| n.is_finite())
    {
        let resets_at = individual["resetsAt"]
            .as_f64()
            .and_then(iso_from_epoch_seconds);
        windows.push(ServerProviderUsageWindow {
            id: "individual".into(),
            kind: UsageWindowKind::Monthly,
            label: "Monthly credit limit".into(),
            used_percent: clamp_percent(100.0 - remaining),
            resets_at,
            window_duration_mins: None,
            estimated: None,
        });
    }
    windows
}

fn codex_reset_credits_to_contract(summary: Option<&Value>) -> Option<ServerProviderResetCredits> {
    let summary = summary?.as_object()?;
    let available_count = summary.get("availableCount").and_then(Value::as_u64)?;
    let next_expires_at = summary
        .get("credits")
        .and_then(Value::as_array)
        .and_then(|credits| {
            credits
                .iter()
                .filter(|credit| credit["status"] == "available")
                .filter_map(|credit| credit["expiresAt"].as_f64())
                .reduce(f64::min)
        })
        .and_then(iso_from_epoch_seconds);
    Some(ServerProviderResetCredits {
        available_count,
        next_expires_at,
        next_credit_id: None,
        options: None,
    })
}

/// Ported from KSG Code's `codexRateLimitsToLimits` (`apps/server/src/provider/codexUsageLimits.ts`).
pub fn codex_rate_limits_to_limits(
    snapshot: &Value,
    rate_limits_by_limit_id: Option<&Value>,
    reset_credits: Option<&Value>,
    checked_at: &str,
) -> ServerProviderUsageLimits {
    let effective_snapshot = rate_limits_by_limit_id
        .and_then(|map| map.get("codex"))
        .filter(|v| v.is_object())
        .unwrap_or(snapshot);
    let windows = codex_rate_limits_to_windows(effective_snapshot);
    let reset_credits = codex_reset_credits_to_contract(reset_credits);
    ServerProviderUsageLimits {
        checked_at: checked_at.to_owned(),
        windows,
        credential_fingerprint: None,
        reset_credits,
        external_usage: None,
        unavailable: None,
    }
}

fn to_title_case_words(value: &str) -> String {
    value
        .split(|c: char| c.is_whitespace() || c == '_' || c == '-')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => {
                    format!("{}{}", first.to_uppercase(), chars.as_str().to_lowercase())
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn claude_subscription_label(subscription_type: &str) -> Option<String> {
    let normalized: String = subscription_type
        .to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '_' && *c != '-')
        .collect();
    if normalized.is_empty() {
        return None;
    }
    Some(match normalized.as_str() {
        "claudemaxsubscription" | "claudemax5xsubscription" | "max" | "maxplan" | "max5" => {
            "Max".into()
        }
        "claudemax20xsubscription" | "max20" => "Ultra".into(),
        "claudeenterprisesubscription" | "enterprise" => "Enterprise".into(),
        "claudeteamsubscription" | "team" => "Team".into(),
        "claudeprosubscription" | "pro" => "Pro".into(),
        "claudefreesubscription" | "free" => "Free".into(),
        _ => to_title_case_words(subscription_type),
    })
}

/// Ported from KSG Code's `claudeAuthMetadata` + `apiProviderAuthMetadata` (`ClaudeProvider.ts`).
pub fn claude_auth_label(
    subscription_type: Option<&str>,
    token_source: Option<&str>,
    api_provider: Option<&str>,
) -> Option<String> {
    if api_provider == Some("bedrock") {
        return Some("Amazon Bedrock".into());
    }
    if let Some(source) = token_source {
        let normalized: String = source
            .to_lowercase()
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '_' && *c != '-')
            .collect();
        if matches!(
            normalized.as_str(),
            "apikey" | "anthropicapikey" | "anthropicauthtoken"
        ) {
            return Some("Claude API Key".into());
        }
    }
    let sub = subscription_type?;
    let label = claude_subscription_label(sub)?;
    let normalized: String = label
        .to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '_' && *c != '-')
        .collect();
    if normalized.starts_with("claude") && normalized.ends_with("subscription") {
        Some(label)
    } else if normalized.starts_with("claude") {
        Some(format!("{label} Subscription"))
    } else if normalized.ends_with("subscription") {
        Some(format!("Claude {label}"))
    } else {
        Some(format!("Claude {label} Subscription"))
    }
}

fn iso_from_str(value: Option<&str>) -> Option<String> {
    let trimmed = value?.trim();
    if trimmed.is_empty() {
        return None;
    }
    DateTime::parse_from_rfc3339(trimmed).ok().map(|dt| {
        dt.with_timezone(&Utc)
            .to_rfc3339_opts(SecondsFormat::Millis, true)
    })
}

fn scoped_window_id(display_name: &str) -> String {
    let mut slug = String::from("seven_day_");
    let mut last_underscore = true;
    for ch in display_name.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            last_underscore = false;
        } else if !last_underscore {
            slug.push('_');
            last_underscore = true;
        }
    }
    while slug.ends_with('_') && slug.len() > "seven_day_".len() {
        slug.pop();
    }
    slug
}

fn percent_from_amounts(used: Option<f64>, limit: Option<f64>) -> Option<f64> {
    let used = used?;
    let limit = limit?;
    if !used.is_finite() || !limit.is_finite() || used < 0.0 || limit <= 0.0 {
        return None;
    }
    let percent = (used / limit) * 100.0;
    percent.is_finite().then(|| clamp_percent(percent))
}

fn read_claude_spend_percent(rate_limits: &Value) -> Option<f64> {
    let spend = &rate_limits["spend"];
    if spend["enabled"].as_bool() == Some(true) {
        let u_curr = spend["used"]["currency"].as_str().map(str::trim);
        let l_curr = spend["limit"]["currency"].as_str().map(str::trim);
        let u_exp = spend["used"]["exponent"].as_u64();
        let l_exp = spend["limit"]["exponent"].as_u64();
        if let (Some(uc), Some(lc), Some(ue), Some(le)) = (u_curr, l_curr, u_exp, l_exp)
            && !uc.is_empty()
            && uc == lc
            && ue == le
            && let Some(pct) = percent_from_amounts(
                spend["used"]["amount_minor"].as_f64(),
                spend["limit"]["amount_minor"].as_f64(),
            )
        {
            return Some(pct);
        }
    }
    let extra = &rate_limits["extra_usage"];
    if extra["is_enabled"].as_bool() != Some(true) {
        return None;
    }
    percent_from_amounts(
        extra["used_credits"].as_f64(),
        extra["monthly_limit"].as_f64(),
    )
    .or_else(|| {
        extra["utilization"]
            .as_f64()
            .filter(|n| n.is_finite() && *n >= 0.0)
            .map(clamp_percent)
    })
}

/// Ported from KSG Code's `claudeUsageResponseToLimits` (`apps/server/src/provider/claudeUsageLimits.ts`).
pub fn claude_usage_response_to_limits(
    response: &Value,
    checked_at: &str,
) -> ServerProviderUsageLimits {
    let available = response["rate_limits_available"].as_bool().unwrap_or(false);
    let rate_limits = &response["rate_limits"];
    if !available || !rate_limits.is_object() {
        return ServerProviderUsageLimits {
            checked_at: checked_at.to_owned(),
            windows: Vec::new(),
            credential_fingerprint: None,
            reset_credits: None,
            external_usage: None,
            unavailable: Some(UsageUnavailable {
                reason: if available {
                    UsageUnavailableReason::ProbeFailed
                } else {
                    UsageUnavailableReason::Unsupported
                },
                message: None,
            }),
        };
    }
    let mut windows = Vec::new();
    for (id, kind, label, mins) in [
        (
            "five_hour",
            UsageWindowKind::Session,
            "Session",
            SESSION_MINS,
        ),
        ("seven_day", UsageWindowKind::Weekly, "Weekly", WEEK_MINS),
    ] {
        let window = &rate_limits[id];
        if let Some(utilization) = window["utilization"].as_f64().filter(|n| n.is_finite()) {
            windows.push(ServerProviderUsageWindow {
                id: id.into(),
                kind,
                label: label.into(),
                used_percent: clamp_percent(utilization),
                resets_at: iso_from_str(window["resets_at"].as_str()),
                window_duration_mins: Some(mins),
                estimated: None,
            });
        }
    }
    if let Some(scoped) = rate_limits["model_scoped"].as_array() {
        for entry in scoped {
            let Some(display_name) = entry["display_name"].as_str() else {
                continue;
            };
            let Some(utilization) = entry["utilization"].as_f64().filter(|n| n.is_finite()) else {
                continue;
            };
            windows.push(ServerProviderUsageWindow {
                id: scoped_window_id(display_name),
                kind: UsageWindowKind::Weekly,
                label: format!("Weekly · {display_name}"),
                used_percent: clamp_percent(utilization),
                resets_at: iso_from_str(entry["resets_at"].as_str()),
                window_duration_mins: Some(WEEK_MINS),
                estimated: None,
            });
        }
    }
    if let Some(spend_percent) = read_claude_spend_percent(rate_limits) {
        windows.push(ServerProviderUsageWindow {
            id: "monthly_spend".into(),
            kind: UsageWindowKind::Monthly,
            label: "Monthly spend".into(),
            used_percent: spend_percent,
            resets_at: None,
            window_duration_mins: None,
            estimated: None,
        });
    }
    ServerProviderUsageLimits {
        checked_at: checked_at.to_owned(),
        windows,
        credential_fingerprint: None,
        reset_credits: None,
        external_usage: None,
        unavailable: None,
    }
}

/// Convert Shina's accumulated `usageLimits` (`{windows:[{id,name,usedPercent,resetsAt}]}`) into
/// `ServerProviderUsageLimits` when a live `rate_limit_event` arrived.
pub fn claude_stream_windows_to_limits(
    usage_limits: &Value,
    checked_at: &str,
) -> Option<ServerProviderUsageLimits> {
    let raw_windows = usage_limits["windows"].as_array()?;
    if raw_windows.is_empty() {
        return None;
    }
    let mut windows = Vec::new();
    for raw in raw_windows {
        let Some(id) = raw["id"].as_str() else {
            continue;
        };
        let Some(used_percent) = raw["usedPercent"].as_f64().filter(|n| n.is_finite()) else {
            continue;
        };
        let (kind, label, mins) = match id {
            "five_hour" => (
                UsageWindowKind::Session,
                "Session".to_owned(),
                Some(SESSION_MINS),
            ),
            "seven_day" => (
                UsageWindowKind::Weekly,
                "Weekly".to_owned(),
                Some(WEEK_MINS),
            ),
            _ => (
                UsageWindowKind::Other,
                raw["name"].as_str().unwrap_or(id).to_owned(),
                None,
            ),
        };
        let resets_at = raw["resetsAt"]
            .as_f64()
            .and_then(iso_from_epoch_seconds)
            .or_else(|| iso_from_str(raw["resetsAt"].as_str()));
        windows.push(ServerProviderUsageWindow {
            id: id.into(),
            kind,
            label,
            used_percent: clamp_percent(used_percent),
            resets_at,
            window_duration_mins: mins,
            estimated: None,
        });
    }
    if windows.is_empty() {
        return None;
    }
    Some(ServerProviderUsageLimits {
        checked_at: checked_at.to_owned(),
        windows,
        credential_fingerprint: None,
        reset_credits: None,
        external_usage: None,
        unavailable: None,
    })
}

fn codex_stream_windows_to_limits(
    usage_limits: &Value,
    checked_at: &str,
) -> Option<ServerProviderUsageLimits> {
    let raw_windows = usage_limits["windows"].as_array()?;
    if raw_windows.is_empty() {
        return None;
    }
    let mut windows = Vec::new();
    for raw in raw_windows {
        let Some(raw_id) = raw["id"].as_str() else {
            continue;
        };
        let Some(used_percent) = raw["usedPercent"].as_f64().filter(|n| n.is_finite()) else {
            continue;
        };
        let id = raw_id.strip_prefix("codex:").unwrap_or(raw_id);
        let (kind, label, mins) = match id {
            "primary" => (
                UsageWindowKind::Session,
                "Session".to_owned(),
                Some(SESSION_MINS),
            ),
            "secondary" => (
                UsageWindowKind::Weekly,
                "Weekly".to_owned(),
                Some(WEEK_MINS),
            ),
            "individual" => (
                UsageWindowKind::Monthly,
                "Monthly credit limit".to_owned(),
                None,
            ),
            _ => (
                UsageWindowKind::Other,
                raw["name"].as_str().unwrap_or(id).to_owned(),
                None,
            ),
        };
        let resets_at = raw["resetsAt"]
            .as_f64()
            .and_then(iso_from_epoch_seconds)
            .or_else(|| iso_from_str(raw["resetsAt"].as_str()));
        windows.push(ServerProviderUsageWindow {
            id: id.into(),
            kind,
            label,
            used_percent: clamp_percent(used_percent),
            resets_at,
            window_duration_mins: mins,
            estimated: None,
        });
    }
    if windows.is_empty() {
        return None;
    }
    Some(ServerProviderUsageLimits {
        checked_at: checked_at.to_owned(),
        windows,
        credential_fingerprint: None,
        reset_credits: None,
        external_usage: None,
        unavailable: None,
    })
}

pub struct UsageLimitsReportInput<'a> {
    pub harness: &'a str,
    pub now_ms: i64,
    pub usage_limits: Option<&'a Value>,
    pub usage_limits_raw: Option<&'a Value>,
    pub usage_account: Option<&'a Value>,
}

/// Ported from KSG Code's `collectProviderUsageLimits` (`packages/shared/src/usageLimits.ts`).
/// Returns `None` for harnesses without usage-limits reports (e.g. Pi, ACP).
pub fn build_usage_limits_report(input: UsageLimitsReportInput<'_>) -> Option<UsageLimitsReport> {
    if !has_usage_limits(input.harness) {
        return None;
    }
    let created_at = iso_from_epoch_ms(input.now_ms);
    let checked_at = input
        .usage_limits_raw
        .and_then(|v| v["checkedAt"].as_str())
        .and_then(|s| iso_from_str(Some(s)))
        .or_else(|| {
            input
                .usage_limits_raw
                .and_then(|v| v["checkedAtMs"].as_i64())
                .map(iso_from_epoch_ms)
        })
        .unwrap_or_else(|| created_at.clone());

    let account = match input.harness {
        "codex" => {
            let raw = input.usage_limits_raw;
            let acct = input.usage_account;
            let snapshot = raw.map(|v| &v["snapshot"]).filter(|v| v.is_object());
            let by_id = raw
                .map(|v| &v["rateLimitsByLimitId"])
                .filter(|v| v.is_object());
            let reset_credits = raw
                .map(|v| &v["rateLimitResetCredits"])
                .filter(|v| v.is_object());
            let limits = if let Some(snapshot) = snapshot {
                codex_rate_limits_to_limits(snapshot, by_id, reset_credits, &checked_at)
            } else if let Some(streamed) = input
                .usage_limits
                .and_then(|v| codex_stream_windows_to_limits(v, &checked_at))
            {
                streamed
            } else {
                ServerProviderUsageLimits {
                    checked_at: checked_at.clone(),
                    windows: Vec::new(),
                    credential_fingerprint: None,
                    reset_credits: None,
                    external_usage: None,
                    unavailable: Some(UsageUnavailable {
                        reason: UsageUnavailableReason::ProbeFailed,
                        message: None,
                    }),
                }
            };
            let usage_plan_type = snapshot
                .and_then(|s| s["planType"].as_str())
                .or_else(|| by_id.and_then(|m| m["codex"]["planType"].as_str()));
            let account_obj = acct.map(|a| {
                if a.get("account").is_some_and(Value::is_object) {
                    &a["account"]
                } else {
                    a
                }
            });
            let plan = codex_account_auth_label(account_obj, usage_plan_type)
                .or_else(|| codex_plan_label(usage_plan_type).map(str::to_owned));
            let email = account_obj
                .filter(|a| a["type"].as_str().is_none_or(|t| t == "chatgpt"))
                .and_then(|a| {
                    let raw = a["email"].as_str()?;
                    if a["emailMasked"].as_bool() == Some(true) {
                        (!raw.trim().is_empty()).then(|| raw.trim().to_owned())
                    } else {
                        conceal_account_identifier(raw)
                    }
                });
            let reset_credit_input = Some(ProviderConsumeResetCreditInput::Instance {
                instance_id: "codex".into(),
                credit_id: None,
            });
            UsageLimitsAccount {
                id: "codex".into(),
                driver: "codex".into(),
                label: "Codex [codex]".into(),
                plan,
                email,
                source_label: None,
                instance_id: Some("codex".into()),
                reset_credit_input,
                display_name: Some("Codex".into()),
                accent_color: None,
                limits,
            }
        }
        "claude" => {
            let raw = input.usage_limits_raw;
            let acct = input.usage_account;
            let mut limits =
                if let Some(response) = raw.map(|v| &v["response"]).filter(|v| v.is_object()) {
                    claude_usage_response_to_limits(response, &checked_at)
                } else if let Some(streamed) = input
                    .usage_limits
                    .and_then(|v| claude_stream_windows_to_limits(v, &checked_at))
                {
                    streamed
                } else {
                    ServerProviderUsageLimits {
                        checked_at: checked_at.clone(),
                        windows: Vec::new(),
                        credential_fingerprint: None,
                        reset_credits: None,
                        external_usage: None,
                        unavailable: Some(UsageUnavailable {
                            reason: UsageUnavailableReason::ProbeFailed,
                            message: None,
                        }),
                    }
                };
            // Merge any newer live rate_limit_event windows if get_usage had none or partial windows.
            if let Some(streamed) = input
                .usage_limits
                .and_then(|v| claude_stream_windows_to_limits(v, &checked_at))
            {
                if limits.unavailable.is_some() {
                    limits.unavailable = None;
                }
                for w in streamed.windows {
                    if let Some(existing) = limits.windows.iter_mut().find(|e| e.id == w.id) {
                        *existing = w;
                    } else {
                        limits.windows.push(w);
                    }
                }
            }
            let plan = acct.and_then(|a| {
                claude_auth_label(
                    a["subscriptionType"].as_str(),
                    a["tokenSource"].as_str(),
                    a["apiProvider"].as_str(),
                )
            });
            let email = acct.and_then(|a| {
                let raw = a["email"].as_str()?;
                if a["emailMasked"].as_bool() == Some(true) {
                    (!raw.trim().is_empty()).then(|| raw.trim().to_owned())
                } else {
                    conceal_account_identifier(raw)
                }
            });
            UsageLimitsAccount {
                id: "claude".into(),
                driver: "claudeAgent".into(),
                label: "Claude [claude]".into(),
                plan,
                email,
                source_label: None,
                instance_id: Some("claude".into()),
                reset_credit_input: Some(ProviderConsumeResetCreditInput::Instance {
                    instance_id: "claude".into(),
                    credit_id: None,
                }),
                display_name: Some("Claude".into()),
                accent_color: None,
                limits,
            }
        }
        _ => return None,
    };

    Some(UsageLimitsReport {
        created_at,
        accounts: vec![account],
        notices: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacted_placeholder_masks_deterministically_and_preserves_delimiters() {
        let masked = redacted_placeholder("alice.smith@example.com");
        assert_ne!(masked, "alice.smith@example.com");
        assert!(!masked.contains("alice"));
        assert!(!masked.contains("example"));
        assert_eq!(masked.chars().nth(5), Some('.'));
        assert_eq!(masked.chars().nth(11), Some('@'));
        assert_eq!(masked.chars().nth(19), Some('.'));
        assert_eq!(masked.len(), "alice.smith@example.com".len());
        assert_eq!(masked, redacted_placeholder("alice.smith@example.com"));
    }

    #[test]
    fn codex_report_builder_maps_windows_plan_reset_credits_and_masks_email() {
        let report = build_usage_limits_report(UsageLimitsReportInput {
            harness: "codex",
            now_ms: 1_700_000_000_000,
            usage_limits: None,
            usage_limits_raw: Some(&json!({
                "checkedAtMs": 1_700_000_000_000i64,
                "snapshot": {
                    "limitId": "codex",
                    "planType": "pro",
                    "primary": {"usedPercent": 42.0, "windowDurationMins": 300, "resetsAt": 1_700_003_600},
                    "secondary": {"usedPercent": 15.0, "windowDurationMins": 10080, "resetsAt": 1_700_600_000}
                },
                "rateLimitResetCredits": {
                    "availableCount": 2,
                    "credits": [{"status": "available", "expiresAt": 1_701_000_000}]
                }
            })),
            usage_account: Some(&json!({
                "account": {
                    "type": "chatgpt",
                    "email": "captain@example.com",
                    "planType": "plus"
                }
            })),
        })
        .expect("codex report");

        assert_eq!(report.accounts.len(), 1);
        let acc = &report.accounts[0];
        assert_eq!(acc.driver, "codex");
        assert_eq!(acc.plan.as_deref(), Some("ChatGPT Pro 200 Subscription"));
        assert_eq!(
            acc.email.as_deref(),
            Some(redacted_placeholder("captain@example.com").as_str())
        );
        assert!(
            !serde_json::to_string(&report)
                .unwrap()
                .contains("captain@example.com")
        );
        assert_eq!(acc.limits.windows.len(), 2);
        assert_eq!(acc.limits.windows[0].label, "Session");
        assert_eq!(acc.limits.windows[0].kind, UsageWindowKind::Session);
        assert_eq!(acc.limits.windows[0].used_percent, 42.0);
        assert_eq!(acc.limits.windows[1].label, "Weekly");
        assert_eq!(acc.limits.windows[1].kind, UsageWindowKind::Weekly);
        assert_eq!(
            acc.limits.reset_credits.as_ref().unwrap().available_count,
            2
        );
    }

    #[test]
    fn claude_report_builder_maps_get_usage_plan_and_masks_email() {
        let report = build_usage_limits_report(UsageLimitsReportInput {
            harness: "claude",
            now_ms: 1_700_000_000_000,
            usage_limits: None,
            usage_limits_raw: Some(&json!({
                "checkedAtMs": 1_700_000_000_000i64,
                "response": {
                    "rate_limits_available": true,
                    "rate_limits": {
                        "five_hour": {"utilization": 65.0, "resets_at": "2023-11-14T23:00:00Z"},
                        "seven_day": {"utilization": 20.0, "resets_at": "2023-11-20T00:00:00Z"},
                        "model_scoped": [{"display_name": "Sonnet", "utilization": 10.0, "resets_at": null}],
                        "extra_usage": {"is_enabled": true, "used_credits": 25.0, "monthly_limit": 100.0}
                    }
                }
            })),
            usage_account: Some(&json!({
                "email": "claude.user@example.com",
                "subscriptionType": "claude_max_20x_subscription",
                "tokenSource": "oauth"
            })),
        })
        .expect("claude report");

        assert_eq!(report.accounts.len(), 1);
        let acc = &report.accounts[0];
        assert_eq!(acc.driver, "claudeAgent");
        assert_eq!(acc.plan.as_deref(), Some("Claude Ultra Subscription"));
        assert_eq!(
            acc.email.as_deref(),
            Some(redacted_placeholder("claude.user@example.com").as_str())
        );
        assert!(
            !serde_json::to_string(&report)
                .unwrap()
                .contains("claude.user@example.com")
        );
        assert_eq!(acc.limits.windows.len(), 4);
        assert_eq!(acc.limits.windows[0].label, "Session");
        assert_eq!(acc.limits.windows[1].label, "Weekly");
        assert_eq!(acc.limits.windows[2].label, "Weekly · Sonnet");
        assert_eq!(acc.limits.windows[3].label, "Monthly spend");
        assert_eq!(acc.limits.windows[3].used_percent, 25.0);
    }

    #[test]
    fn unsupported_harnesses_return_no_report() {
        for harness in ["pi", "opencode", "gemini", "claude-code-acp"] {
            assert!(
                build_usage_limits_report(UsageLimitsReportInput {
                    harness,
                    now_ms: 1_700_000_000_000,
                    usage_limits: None,
                    usage_limits_raw: None,
                    usage_account: None,
                })
                .is_none()
            );
        }
    }
}
