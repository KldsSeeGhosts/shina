//! Pi's live context-window usage for one thread.
//!
//! `get_session_stats` reports two different token figures. `data.tokens` is cumulative
//! session usage across every assistant message, tool and compaction. `data.contextUsage`
//! is the current context window estimate Pi uses for compaction and its footer. Shina
//! reads only the latter, so cumulative session totals cannot overstate current usage.
//!
//! Pi reports `contextUsage.tokens` and `.percent` as null immediately after compaction,
//! until a fresh assistant response supplies usage. Those nulls stay unknown here; they
//! never become a stale percentage or a zero.

use serde_json::Value;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ContextPhase {
    #[default]
    Idle,
    Compacting,
    AwaitingUsage,
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ContextUsage {
    pub tokens: Option<u64>,
    pub context_window: Option<u64>,
    pub percent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub breakdown: Option<Vec<crate::context_breakdown::ContextBreakdownRow>>,
}

impl ContextUsage {
    /// Reads only `contextUsage` from a `get_session_stats` response.
    pub fn from_stats(data: &Value) -> Self {
        let usage = &data["contextUsage"];
        if !usage.is_object() {
            return Self::default();
        }
        let context_window = count(&usage["contextWindow"]).filter(|window| *window > 0);
        let tokens = count(&usage["tokens"]);
        let percent = match usage.get("percent") {
            Some(percent) => percent
                .as_f64()
                .filter(|percent| percent.is_finite() && *percent >= 0.0),
            None => {
                // Pi normally sends percent. Derive it from the same live figures only when
                // the field is absent, never from cumulative session tokens.
                match (tokens, context_window) {
                    (Some(tokens), Some(window)) => Some(tokens as f64 * 100.0 / window as f64),
                    _ => None,
                }
            }
        };
        let breakdown = usage
            .get("breakdown")
            .and_then(|value| {
                serde_json::from_value::<Vec<crate::context_breakdown::ContextBreakdownRow>>(
                    value.clone(),
                )
                .ok()
            })
            .filter(|rows| !rows.is_empty());
        Self {
            tokens,
            context_window,
            percent,
            breakdown,
        }
    }

    /// Whether Pi supplied a percentage for the current context.
    pub fn has_usage(&self) -> bool {
        self.percent.is_some()
    }

    /// The limit Pi reported, or the selected model's known capacity while Pi is silent.
    pub fn limit(&self, model_window: Option<u64>) -> Option<u64> {
        self.context_window.or(model_window)
    }

    pub fn percent_text(&self) -> Option<String> {
        self.percent
            .map(|percent| format!("{}%", percent.round() as i64))
    }

    /// Used and limit tokens, with the model's capacity filling in when Pi omitted it.
    pub fn usage_text(&self, model_window: Option<u64>) -> String {
        let limit = self.limit(model_window);
        match (self.tokens, limit) {
            (Some(tokens), Some(limit)) => {
                format!("{} of {} tokens", count_text(tokens), count_text(limit))
            }
            (Some(tokens), None) => format!("{} tokens", count_text(tokens)),
            (None, Some(limit)) => format!("Limit {} tokens", count_text(limit)),
            (None, None) => "Not reported".into(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ContextMeter {
    pub usage: ContextUsage,
    pub phase: ContextPhase,
    /// The usage popover was opened with a press, Enter or Space.
    pub details_open: bool,
    /// The pointer rests on the ring, or on the popover the ring opened.
    pub ring_hovered: bool,
    pub popover_hovered: bool,
    /// The id of the in-flight `get_session_stats` request. Responses to an older request
    /// are ignored, which is how a stats request that crossed a compaction cannot restore
    /// the percentage compaction invalidated.
    pending: Option<String>,
}

impl ContextMeter {
    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Records the request id returned by `Thread::command`.
    pub fn requested(&mut self, id: String) {
        self.pending = Some(id);
    }

    pub fn cancel_request(&mut self) {
        self.pending = None;
    }

    /// Whether `id` is the request this meter is waiting for.
    pub fn accepts(&self, id: Option<&str>) -> bool {
        id.is_some() && self.pending.as_deref() == id
    }

    /// Applies a stats response only when it belongs to the current request.
    pub fn accept_response(&mut self, record: &Value) -> bool {
        if !self.accepts(record["id"].as_str()) {
            return false;
        }
        self.pending = None;
        if record["success"] == false {
            return false;
        }
        self.absorb(&record["data"]);
        true
    }

    /// Replaces usage with Pi's current figures. A missing or null percentage keeps the
    /// unknown state instead of reusing the previous percentage.
    pub fn absorb(&mut self, data: &Value) {
        self.usage = ContextUsage::from_stats(data);
        if self.usage.has_usage() || self.phase == ContextPhase::Idle {
            self.phase = ContextPhase::Idle;
        } else {
            self.phase = ContextPhase::AwaitingUsage;
        }
    }

    pub fn begin_compaction(&mut self) {
        self.pending = None;
        self.phase = ContextPhase::Compacting;
        self.usage.tokens = None;
        self.usage.percent = None;
        self.usage.breakdown = None;
    }

    pub fn finish_compaction(&mut self) {
        self.pending = None;
        self.phase = ContextPhase::AwaitingUsage;
        self.usage.tokens = None;
        self.usage.percent = None;
        self.usage.breakdown = None;
    }

    /// A different model has different capacity and a different cache. Do not show the
    /// previous model's percentage or window while Pi measures the new one.
    pub fn begin_model_change(&mut self) {
        self.pending = None;
        self.usage = ContextUsage::default();
        self.phase = ContextPhase::Idle;
    }

    pub fn state_text(&self) -> Option<&'static str> {
        match self.phase {
            ContextPhase::Compacting => Some("Compacting context"),
            ContextPhase::AwaitingUsage => Some("Waiting for Pi to report usage"),
            ContextPhase::Idle if self.is_pending() => Some("Refreshing context"),
            ContextPhase::Idle => None,
        }
    }

    /// What assistive technology hears on the ring's button.
    pub fn accessible_label(&self, model_window: Option<u64>) -> String {
        if let Some(percent) = &self.usage.percent_text() {
            return format!(
                "Context usage: {}, {percent}",
                self.usage.usage_text(model_window)
            );
        }
        match self.phase {
            ContextPhase::Compacting => "Context usage unknown while Pi compacts".into(),
            ContextPhase::AwaitingUsage => {
                "Context usage unknown after compaction. View context details".into()
            }
            ContextPhase::Idle if self.is_pending() => {
                "Context usage refreshing. View context details".into()
            }
            ContextPhase::Idle => "Context usage unavailable. View context details".into(),
        }
    }

    /// Whether the usage popover shows: opened from the keyboard or a press, or hovered.
    pub fn popover_open(&self) -> bool {
        self.details_open || self.ring_hovered || self.popover_hovered
    }

    /// Records the pointer entering or leaving the ring (`ring`) or its popover. Once the
    /// pointer has left both, the popover closes, including one a press opened.
    pub fn hover(&mut self, ring: bool, inside: bool) {
        if ring {
            self.ring_hovered = inside;
        } else {
            self.popover_hovered = inside;
        }
        if !self.ring_hovered && !self.popover_hovered && !inside {
            self.details_open = false;
        }
    }

    pub fn close_popover(&mut self) {
        self.details_open = false;
        self.ring_hovered = false;
        self.popover_hovered = false;
    }
}

fn count(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| {
        value
            .as_f64()
            .filter(|number| number.is_finite() && *number >= 0.0 && *number <= u64::MAX as f64)
            .map(|number| number as u64)
    })
}

pub fn count_text(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, byte) in digits.bytes().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(byte as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn stats_use_the_live_context_and_ignore_cumulative_session_tokens() {
        let data = json!({
            "tokens": {"input": 50000, "output": 10000, "cacheRead": 40000, "cacheWrite": 5000, "total": 105000},
            "cost": 0.45,
            "contextUsage": {"tokens": 60000, "contextWindow": 200000, "percent": 30}
        });
        let usage = ContextUsage::from_stats(&data);
        assert_eq!(usage.tokens, Some(60000));
        assert_eq!(usage.context_window, Some(200000));
        assert_eq!(usage.percent, Some(30.0));
        assert_eq!(usage.percent_text().as_deref(), Some("30%"));
        assert_eq!(
            ContextUsage {
                tokens: Some(60000),
                context_window: Some(200000),
                percent: Some(30.0),
                breakdown: None,
            }
            .usage_text(None),
            "60,000 of 200,000 tokens"
        );
    }

    #[test]
    fn missing_context_usage_and_nulls_stay_unknown() {
        let missing = ContextUsage::from_stats(&json!({"tokens": {"total": 900000}}));
        assert_eq!(missing, ContextUsage::default());
        let nulls = ContextUsage::from_stats(&json!({
            "contextUsage": {"tokens": null, "contextWindow": null, "percent": null}
        }));
        assert_eq!(nulls, ContextUsage::default());
        assert!(!nulls.has_usage());
    }

    #[test]
    fn zero_window_never_becomes_a_fake_zero_percentage() {
        let zero_window = ContextUsage::from_stats(&json!({
            "contextUsage": {"tokens": 0, "contextWindow": 0, "percent": null}
        }));
        assert_eq!(zero_window.tokens, Some(0));
        assert_eq!(zero_window.context_window, None);
        assert_eq!(zero_window.percent, None);
        assert!(!zero_window.has_usage());
        assert_eq!(zero_window.usage_text(Some(200000)), "0 of 200,000 tokens");
    }

    #[test]
    fn an_explicit_zero_is_real_usage() {
        let usage = ContextUsage::from_stats(&json!({
            "contextUsage": {"tokens": 0, "contextWindow": 200000, "percent": 0}
        }));
        assert!(usage.has_usage());
        assert_eq!(usage.percent, Some(0.0));
        assert_eq!(usage.percent_text().as_deref(), Some("0%"));
    }

    #[test]
    fn percent_falls_back_to_live_tokens_not_session_totals() {
        let usage = ContextUsage::from_stats(&json!({
            "tokens": {"total": 500000},
            "contextUsage": {"tokens": 5000, "contextWindow": 20000}
        }));
        assert_eq!(usage.percent, Some(25.0));
        assert_eq!(usage.tokens, Some(5000));
    }

    #[test]
    fn an_explicit_null_percent_is_unknown_even_when_tokens_are_present() {
        let usage = ContextUsage::from_stats(&json!({
            "contextUsage": {"tokens": 6000, "contextWindow": 20000, "percent": null}
        }));
        assert_eq!(usage.tokens, Some(6000));
        assert_eq!(usage.percent, None);
        assert!(!usage.has_usage());
    }

    #[test]
    fn a_stats_response_from_before_compaction_cannot_restore_the_old_ring() {
        let mut meter = ContextMeter::default();
        meter.requested("1-1".into());
        meter.accept_response(&json!({
            "id": "1-1", "type": "response", "command": "get_session_stats", "success": true,
            "data": {"contextUsage": {"tokens": 120000, "contextWindow": 200000, "percent": 60}}
        }));
        assert_eq!(meter.usage.percent, Some(60.0));

        meter.begin_compaction();
        assert_eq!(meter.phase, ContextPhase::Compacting);
        assert_eq!(meter.usage.tokens, None);
        assert_eq!(meter.usage.percent, None);
        assert_eq!(meter.usage.context_window, Some(200000));
        assert!(!meter.is_pending());

        // Even if the old response reaches the UI now, it does not match a pending id.
        meter.accept_response(&json!({
            "id": "1-1", "type": "response", "command": "get_session_stats", "success": true,
            "data": {"contextUsage": {"tokens": 120000, "contextWindow": 200000, "percent": 60}}
        }));
        assert_eq!(meter.usage.percent, None);

        meter.finish_compaction();
        assert_eq!(meter.phase, ContextPhase::AwaitingUsage);
        meter.requested("1-2".into());
        meter.accept_response(&json!({
            "id": "1-2", "type": "response", "command": "get_session_stats", "success": true,
            "data": {"contextUsage": {"tokens": null, "contextWindow": 200000, "percent": null}}
        }));
        assert_eq!(meter.phase, ContextPhase::AwaitingUsage);
        assert_eq!(meter.usage.percent, None);
        assert_eq!(meter.usage.context_window, Some(200000));
        assert_eq!(
            meter.accessible_label(None),
            "Context usage unknown after compaction. View context details"
        );
    }

    #[test]
    fn fresh_stats_after_compaction_return_the_ring_to_real_usage() {
        let mut meter = ContextMeter::default();
        meter.begin_compaction();
        meter.finish_compaction();
        meter.requested("2-1".into());
        meter.accept_response(&json!({
            "id": "2-1", "command": "get_session_stats", "success": true,
            "data": {"contextUsage": {"tokens": 30000, "contextWindow": 200000, "percent": 15}}
        }));
        assert_eq!(meter.phase, ContextPhase::Idle);
        assert_eq!(meter.usage.percent, Some(15.0));
    }

    #[test]
    fn model_changes_clear_the_previous_capacity_and_percentage() {
        let mut meter = ContextMeter {
            usage: ContextUsage {
                tokens: Some(60000),
                context_window: Some(200000),
                percent: Some(30.0),
                breakdown: None,
            },
            ..Default::default()
        };
        meter.begin_model_change();
        assert_eq!(meter.usage, ContextUsage::default());
        assert_eq!(meter.phase, ContextPhase::Idle);
        assert_eq!(meter.usage.usage_text(Some(100000)), "Limit 100,000 tokens");
        assert_eq!(
            meter.accessible_label(Some(100000)),
            "Context usage unavailable. View context details"
        );
    }

    #[test]
    fn number_grouping_and_state_text_are_human_readable() {
        assert_eq!(count_text(0), "0");
        assert_eq!(count_text(999), "999");
        assert_eq!(count_text(1000), "1,000");
        assert_eq!(count_text(200000), "200,000");
        assert_eq!(count_text(1234567), "1,234,567");

        let mut meter = ContextMeter::default();
        assert_eq!(meter.state_text(), None);
        meter.requested("1-1".into());
        assert_eq!(meter.state_text(), Some("Refreshing context"));
        meter.begin_compaction();
        assert_eq!(meter.state_text(), Some("Compacting context"));
        meter.finish_compaction();
        assert_eq!(meter.state_text(), Some("Waiting for Pi to report usage"));
    }
}
