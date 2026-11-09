//! Workspace-owned captain profiles. Portrait indices are separate from profile indices.
use crate::visuals;
use rune::TextEdit;
use serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Captain {
    pub name: String,
    pub role: String,
    pub instructions: String,
    pub portrait: usize,
}

impl Captain {
    pub fn defaults() -> Vec<Self> {
        (0..visuals::CAPTAIN_NAMES.len())
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
        if self.role.trim().is_empty()
            || self.role.chars().count() > 100
            || self.role.chars().any(char::is_control)
        {
            return Err("Use a role of 1 to 100 characters on one line.".into());
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
        format!(
            "Shina captain profile\nName: {}\nRole: {}\nWorkspace-specific instructions:\n{}\n\n\
             Follow the workspace's instructions and the user's current request. \
             This profile does not grant tool permissions or access to other agents. \
             Do not claim to have delegated work unless a real delegation tool was used.",
            self.name, self.role, self.instructions
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

pub struct CaptainEditor {
    pub editing: Option<usize>,
    pub name: TextEdit,
    pub role: TextEdit,
    pub instructions: TextEdit,
    pub portrait: usize,
    pub error: Option<String>,
}

impl CaptainEditor {
    pub fn new() -> Self {
        let mut editor = Self {
            editing: None,
            name: TextEdit::default(),
            role: TextEdit::default(),
            instructions: TextEdit::default(),
            portrait: 0,
            error: None,
        };
        editor.name.focused = true;
        editor.role.replace_text("General");
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
        let captain = Captain::defaults().remove(2);
        assert_eq!(Captain::from_value(&captain.to_value()), Some(captain));
        assert!(
            Captain::from_value(
                &json!({"name":"Test","role":"Role","instructions":"","portrait":99})
            )
            .is_none()
        );
        let mut blank = Captain::defaults().remove(0);
        blank.name = " \n".into();
        assert!(blank.validate().is_err());
    }
}
