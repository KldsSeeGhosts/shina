//! Assistant profiles. Portrait indices are separate from stable captain profile indices.
use crate::visuals;
use rune::TextEdit;
use serde_json::{Value, json};

/// How many named presets earlier versions seeded.
const LEGACY_PRESETS: usize = 4;

/// The backend's stale-version refusal for `captain.promote`, which the client
/// classifies as a review-again conflict.
pub const PROMOTE_CONFLICT_PREFIX: &str = "Thread changed since version";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Captain {
    pub name: String,
    pub role: String,
    pub instructions: String,
    pub portrait: usize,
}

impl Captain {
    /// The named roster earlier versions created on first run. New rosters start empty; this
    /// remains for migrating old indexes, recognising untouched presets, and the demo.
    /// Always the original four, even though more portraits ship now: migrations compare
    /// stored rosters against exactly what those versions wrote.
    pub fn presets() -> Vec<Self> {
        (0..LEGACY_PRESETS)
            .map(|portrait| Self {
                name: visuals::CAPTAIN_NAMES[portrait].into(),
                role: visuals::CAPTAIN_ROLES[portrait].into(),
                instructions: visuals::CAPTAIN_ROLE_DETAIL[portrait].into(),
                portrait,
            })
            .collect()
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty()
            || self.name.chars().count() > 60
            || self.name.chars().any(char::is_control)
        {
            return Err("Use a captain name of 1 to 60 characters on one line.".into());
        }
        // The role is optional.
        if self.role.chars().count() > 100 || self.role.chars().any(char::is_control) {
            return Err("Keep the role under 100 characters, on one line.".into());
        }
        if self.instructions.chars().count() > 8000 || self.instructions.contains('\0') {
            return Err(
                "Keep captain instructions under 8,000 characters, without null characters.".into(),
            );
        }
        if self.portrait >= visuals::CAPTAIN_NAMES.len() {
            return Err("Choose one of the available portraits.".into());
        }
        Ok(())
    }

    pub fn prompt(&self) -> String {
        let role = if self.role.trim().is_empty() {
            String::new()
        } else {
            format!("Role: {}\n", self.role)
        };
        format!(
            "Shina captain profile\nName: {}\n{role}Captain instructions:\n{}\n\n\
             Follow applicable workspace instructions and the user's current request. \
             This profile does not grant tool permissions or access to other agents. \
             Do not claim to have delegated work unless a real delegation tool was used.",
            self.name, self.instructions
        )
    }

    pub fn to_value(&self) -> Value {
        json!({"name":self.name,"role":self.role,"instructions":self.instructions,"portrait":self.portrait})
    }

    pub fn from_value(value: &Value) -> Option<Self> {
        let captain = Self {
            name: value["name"].as_str()?.into(),
            role: value["role"].as_str()?.into(),
            instructions: value["instructions"].as_str()?.into(),
            portrait: usize::try_from(value["portrait"].as_u64()?).ok()?,
        };
        captain.validate().ok()?;
        Some(captain)
    }
}

/// Promotion mode state carried by `CaptainEditor` while an existing thread is
/// being made a Captain. The thread identity, machine and workspace are fixed
/// at open time; `preview` is the backend's authoritative `captain.promote.preview`
/// result and `pending` whichever command is in flight (first the preview read,
/// then the mutation). `error` lives on the editor itself.
#[derive(Clone, Debug)]
pub struct CaptainPromotion {
    /// The machine key the dialog opened on; it never changes for the dialog's life.
    pub server: String,
    /// The server-side thread id on that machine.
    pub thread: usize,
    /// The thread's title at open time, for the dialog title and toasts.
    pub title: String,
    /// The thread's workspace at open time. Promotion never moves a thread.
    pub project: Option<usize>,
    /// The retired Captain whose profile will be restored. `Some` only while the
    /// restore switch is on; the UI locks the profile fields while it is `Some`.
    pub restore: Option<usize>,
    /// The retired Captain eligible for restore, present when this thread is its
    /// surviving former home. `restore` is never `Some` without this.
    pub restore_candidate: Option<usize>,
    /// The backend's own read shape (`threadId`, `restoreCaptain`,
    /// `threadVersion`, `blockers`, `permissionsChanged`).
    pub preview: Option<Value>,
    /// The `captain.promote.preview` read or `captain.promote` mutation in flight.
    pub pending: Option<uuid::Uuid>,
}

impl CaptainPromotion {
    /// The `restoreCaptain` argument for preview and promote commands.
    pub fn restore_captain(&self) -> Option<usize> {
        self.restore
    }

    /// The backend refused the last confirmation because the thread changed
    /// under the open editor. Carried on the editor's error line so the user
    /// re-reviews before a fresh press.
    pub fn conflict(error: Option<&str>) -> bool {
        error.is_some_and(|error| error.starts_with(PROMOTE_CONFLICT_PREFIX))
    }
}

pub struct CaptainEditor {
    pub server: String,
    pub machine_picker: bool,
    pub editing: Option<usize>,
    pub promotion: Option<CaptainPromotion>,
    pub name: TextEdit,
    pub role: TextEdit,
    pub instructions: TextEdit,
    pub portrait: usize,
    pub error: Option<String>,
}

impl CaptainEditor {
    pub fn new() -> Self {
        let mut editor = Self {
            server: "local".into(),
            machine_picker: false,
            editing: None,
            promotion: None,
            name: TextEdit::default(),
            role: TextEdit::default(),
            instructions: TextEdit::default(),
            portrait: 0,
            error: None,
        };
        editor.name.focused = true;
        editor
    }

    pub fn profile(&self) -> Captain {
        Captain {
            name: self.name.text.trim().into(),
            role: self.role.text.trim().into(),
            instructions: self.instructions.text.trim().into(),
            portrait: self.portrait,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_round_trip_and_reject_invalid_data() {
        let captain = Captain::presets().remove(2);
        assert_eq!(Captain::from_value(&captain.to_value()), Some(captain));
        assert!(
            Captain::from_value(
                &json!({"name":"Test","role":"Role","instructions":"","portrait":99})
            )
            .is_none()
        );
        let mut blank = Captain::presets().remove(0);
        blank.name = " \n".into();
        assert!(blank.validate().is_err());
    }

    #[test]
    fn every_shipped_portrait_is_valid_and_presets_stay_the_original_four() {
        assert_eq!(Captain::presets().len(), 4);
        let mut captain = Captain::presets().remove(0);
        for portrait in 0..visuals::PORTRAITS {
            captain.portrait = portrait;
            assert!(captain.validate().is_ok(), "portrait {portrait}");
            assert_eq!(
                Captain::from_value(&captain.to_value()),
                Some(captain.clone())
            );
        }
        captain.portrait = visuals::PORTRAITS;
        assert!(captain.validate().is_err());
    }
}
