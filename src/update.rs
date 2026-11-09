//! In-app updates from GitHub Releases. The repository is private, so downloads go through
//! the user's existing `gh` CLI login; Shina never reads or stores a GitHub token.
use rune::Waker;
use serde_json::Value;
use std::{
    path::PathBuf,
    process::Command,
    sync::mpsc::{self, Receiver},
    thread,
};

pub const REPOSITORY: &str = "KldsSeeGhosts/shina";

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// No check yet, or this is a development build.
    Idle,
    Checking,
    UpToDate,
    Available {
        tag: String,
    },
    Downloading {
        tag: String,
    },
    /// The new executable is downloaded and waits for a restart.
    Ready {
        tag: String,
        exe: PathBuf,
    },
    Failed(String),
}

pub struct Updater {
    pub status: Status,
    events: Option<Receiver<Status>>,
}

impl Default for Updater {
    fn default() -> Self {
        Self {
            status: Status::Idle,
            events: None,
        }
    }
}

fn gh() -> Command {
    let mut command = Command::new("gh");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    command
}

/// `v0.2.10` or `0.2.10` as comparable numbers. Anything else is not a release version.
pub fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let mut parts = text.trim().trim_start_matches('v').split('.');
    let version = (
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.split('-').next()?.parse().ok()?,
    );
    parts.next().is_none().then_some(version)
}

pub fn is_newer(tag: &str, current: &str) -> bool {
    matches!((parse_version(tag), parse_version(current)), (Some(a), Some(b)) if a > b)
}

fn latest_tag() -> Result<String, String> {
    let output = gh()
        .args([
            "release",
            "view",
            "--repo",
            REPOSITORY,
            "--json",
            "tagName,assets",
        ])
        .output()
        .map_err(|_| {
            "The GitHub CLI (gh) is not installed, so Shina cannot check for updates.".to_owned()
        })?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        return Err(if detail.contains("release not found") {
            "No Shina release has been published yet.".into()
        } else {
            format!("Could not check for updates: {}", detail.trim())
        });
    }
    let release: Value = serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())?;
    let has_exe = release["assets"]
        .as_array()
        .is_some_and(|assets| assets.iter().any(|a| a["name"] == "shina.exe"));
    if !has_exe {
        return Err("The latest release has no shina.exe.".into());
    }
    release["tagName"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| "The latest release has no tag.".into())
}

fn download(tag: &str) -> Result<PathBuf, String> {
    let dir = crate::install::install_dir()
        .ok_or("LOCALAPPDATA is not set.")?
        .join("download");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let output = gh()
        .args([
            "release",
            "download",
            tag,
            "--repo",
            REPOSITORY,
            "--pattern",
            "shina.exe",
            "--clobber",
            "--dir",
        ])
        .arg(&dir)
        .output()
        .map_err(|e| format!("Could not run gh: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "Could not download {tag}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let exe = dir.join("shina.exe");
    match std::fs::metadata(&exe) {
        Ok(meta) if meta.len() > 1024 * 1024 => Ok(exe),
        _ => Err(format!("The {tag} download is incomplete.")),
    }
}

impl Updater {
    fn spawn(&mut self, wake: Waker, work: impl FnOnce() -> Status + Send + 'static) {
        let (sender, receiver) = mpsc::channel();
        self.events = Some(receiver);
        thread::spawn(move || {
            let _ = sender.send(work());
            wake();
        });
    }

    /// Checks GitHub for a newer release. Development builds never update themselves.
    pub fn check(&mut self, wake: Waker) {
        if !crate::install::is_installed()
            || matches!(
                self.status,
                Status::Checking | Status::Downloading { .. } | Status::Ready { .. }
            )
        {
            return;
        }
        self.status = Status::Checking;
        self.spawn(wake, || match latest_tag() {
            Ok(tag) if is_newer(&tag, env!("CARGO_PKG_VERSION")) => Status::Available { tag },
            Ok(_) => Status::UpToDate,
            Err(error) => Status::Failed(error),
        });
    }

    pub fn download(&mut self, wake: Waker) {
        let Status::Available { tag } = self.status.clone() else {
            return;
        };
        self.status = Status::Downloading { tag: tag.clone() };
        self.spawn(wake, move || match download(&tag) {
            Ok(exe) => Status::Ready { tag, exe },
            Err(error) => Status::Failed(error),
        });
    }

    /// Takes a finished check or download. Returns whether the status changed.
    pub fn poll(&mut self) -> bool {
        let Some(status) = self
            .events
            .as_ref()
            .and_then(|events| events.try_recv().ok())
        else {
            return false;
        };
        self.events = None;
        self.status = status;
        true
    }

    /// Swaps the downloaded executable into place and starts it. The caller quits afterwards.
    pub fn install_and_relaunch(&mut self) -> Result<(), String> {
        let Status::Ready { exe, .. } = &self.status else {
            return Err("No update is ready.".into());
        };
        let installed = crate::install::place_exe(exe)?;
        let _ = std::fs::remove_file(exe);
        // `--install` refreshes the shortcut, app ID and Apps & Features version for the new
        // build, then starts it on the last workspace.
        crate::install::launch_with(&installed, &["--install"])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compares_release_tags_with_the_running_version() {
        assert!(is_newer("v0.2.0", "0.1.9"));
        assert!(is_newer("v0.1.10", "0.1.9"));
        assert!(!is_newer("v0.1.0", "0.1.0"));
        assert!(!is_newer("nightly", "0.1.0"));
        assert_eq!(parse_version("v1.2.3-beta"), Some((1, 2, 3)));
        assert_eq!(parse_version("1.2"), None);
    }
}
