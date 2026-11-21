//! Shared update copy and actions for the footer and Settings.

#[derive(Clone, Debug, PartialEq)]
pub enum UpdatePhase {
    Disabled,
    Idle,
    Checking,
    UpToDate,
    Available {
        tag: String,
        targets: usize,
    },
    Downloading {
        tag: String,
        progress: Option<f32>,
    },
    Ready {
        tag: String,
        blocker: Option<String>,
    },
    Failed {
        tag: Option<String>,
    },
    FleetRunning {
        done: usize,
        total: usize,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateAction {
    CheckNow,
    Restart,
    RestartPostponed,
    Postpone,
    Retry,
    UpdateEverything,
    UpdateMachine(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MachineUpdate {
    pub is_app: bool,
    pub server: String,
    pub name: String,
    pub status: String,
    pub current_version: Option<String>,
    pub target_version: Option<String>,
    pub action: Option<UpdateAction>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct UpdateSummary {
    pub phase: UpdatePhase,
    pub headline: String,
    pub detail: String,
    pub cause: Option<String>,
    pub raw: Vec<String>,
    pub action: Option<UpdateAction>,
    pub machines: Vec<MachineUpdate>,
    pub version: &'static str,
    pub last_check: String,
    pub dismissible: bool,
}

impl UpdateSummary {
    pub fn from_phase(
        phase: UpdatePhase,
        detail: Option<String>,
        raw: Vec<String>,
        machines: Vec<MachineUpdate>,
        last_check: String,
        retry_at: Option<&str>,
        can_retry: bool,
    ) -> Self {
        use UpdateAction as Action;
        use UpdatePhase::*;
        let version = env!("CARGO_PKG_VERSION");
        let (headline, action) = match &phase {
            Disabled => ("Updates are disabled".into(), None),
            Idle => ("Check for updates".into(), Some(Action::CheckNow)),
            Checking => ("Checking for updates".into(), None),
            UpToDate => ("Shina is up to date".into(), Some(Action::CheckNow)),
            Available { .. } => ("Update available".into(), Some(Action::UpdateEverything)),
            Downloading { tag, .. } => (format!("Downloading {tag}"), None),
            Ready { blocker, .. } => (
                "Ready to restart".into(),
                blocker.is_none().then_some(Action::Restart),
            ),
            Failed { .. } => ("Update failed".into(), can_retry.then_some(Action::Retry)),
            FleetRunning { done, total } => (format!("Updating {done} of {total} machines"), None),
        };
        let cause = matches!(phase, Failed { .. })
            .then(|| raw.iter().rev().find_map(|line| cause(line, retry_at)))
            .flatten();
        let detail = detail.unwrap_or_else(|| match &phase {
            Ready { tag, blocker } => blocker
                .clone()
                .unwrap_or_else(|| format!("Restart to update to {tag}.")),
            Failed { tag: Some(_) } => "Shina could not restart to finish the update.".into(),
            Failed { tag: None } => "Shina could not complete the update.".into(),
            _ => format!("Version {version}. {last_check}."),
        });
        Self {
            phase,
            headline,
            detail,
            cause,
            raw,
            action,
            machines,
            version,
            last_check,
            dismissible: false,
        }
    }
}

/// Raw diagnostics stay in Details. Only known failures get a remedy.
pub fn cause(raw: &str, retry_at: Option<&str>) -> Option<String> {
    let raw = raw.to_ascii_lowercase();
    if raw.contains("external effect is still dispatching") {
        Some("A tool was still running. Try again when no thread is working.".into())
    } else if raw.contains("rate limit") || raw.contains("rate-limit") {
        Some(retry_at.map_or_else(
            || "GitHub is limiting requests. Shina will retry later.".into(),
            |at| format!("GitHub is limiting requests. Shina retries at {at}."),
        ))
    } else if raw.contains("authentication denied")
        || raw.contains("authentication denial")
        || raw.contains("authentication failed")
        || raw.contains("gh auth login")
        || raw.contains("bad credentials")
    {
        Some("GitHub sign-in was refused. Run gh auth login, then retry.".into())
    } else if raw.contains("checksum")
        || raw.contains("signature mismatch")
        || raw.contains("sha256 mismatch")
    {
        Some("The download did not verify. Shina will fetch it again.".into())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_failures_have_plain_language_remedies() {
        assert!(
            cause("an external effect is still dispatching", None)
                .unwrap()
                .starts_with("A tool")
        );
        assert_eq!(
            cause("HTTP 429 rate-limit", Some("14:32")).as_deref(),
            Some("GitHub is limiting requests. Shina retries at 14:32.")
        );
        assert!(
            cause("Authentication denied", None)
                .unwrap()
                .contains("gh auth login")
        );
        assert!(
            cause("checksum mismatch", None)
                .unwrap()
                .contains("did not verify")
        );
        assert_eq!(cause("Unexpected end of file", None), None);
    }

    #[test]
    fn failure_cause_and_ready_action_agree_with_the_phase() {
        let summary = |phase| {
            UpdateSummary::from_phase(
                phase,
                None,
                vec!["Checksum mismatch".into()],
                vec![],
                "Not checked this session".into(),
                None,
                true,
            )
        };
        let failure = summary(UpdatePhase::Failed { tag: None });
        assert!(failure.cause.is_some());
        assert_eq!(failure.action, Some(UpdateAction::Retry));
        let ready = summary(UpdatePhase::Ready {
            tag: "v2".into(),
            blocker: Some("While working".into()),
        });
        assert!(ready.cause.is_none());
        assert_eq!(ready.detail, "While working");
        assert_eq!(ready.action, None);
    }
}
