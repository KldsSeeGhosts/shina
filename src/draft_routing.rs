//! Ported from KSG Code's `useLogicalProjectEnvironments` and
//! `deriveLogicalProjectKeyFromSettings` (`projectGrouping.ts`, repository
//! grouping mode): one logical project can exist once per machine, so a draft
//! retarget looks up the target machine's member of the same logical group.
//!
//! Matching key order (KSG's `deriveLogicalProjectKey`):
//! 1. Repository canonical key (`repositoryGroupingKeyOf`: the origin's
//!    canonical remote, else the repository's own canonical remote). On the
//!    wire this is the project's `gitRemote`.
//! 2. Physical key (`derivePhysicalProjectKey`): the normalized workspace
//!    path. KSG scopes it by environment, which can never match across
//!    machines, so a client-side retarget falls back to the workspace's
//!    folder name — the only environment-independent part of the path — for
//!    projects that carry no repository identity.
//! Ties follow KSG's `shouldReplacePhysicalProjectWinner`: the freshest
//! `updated` wins, then the higher project id.
use serde_json::Value;

/// The fields of a project that logical matching needs.
#[derive(Clone, Debug)]
pub struct RouteProject {
    pub id: usize,
    pub path: String,
    pub git_remote: Option<String>,
    pub hidden: bool,
    pub updated: u64,
}

impl RouteProject {
    /// From a wire `index.projects[]` record (`projection_from_snapshot`).
    pub fn from_record(value: &Value) -> Option<Self> {
        Some(Self {
            id: value["id"].as_u64()? as usize,
            path: value["path"].as_str()?.to_owned(),
            git_remote: value["gitRemote"].as_str().map(str::to_owned),
            hidden: value["hidden"] == true,
            updated: value["updated"].as_u64().unwrap_or(0),
        })
    }
}

/// KSG's `canonicalKey`: a remote normalized so the same repository compares
/// equal across `git@host:`, `ssh://`, `https://` and trailing `.git` forms.
pub fn canonical_remote(remote: &str) -> String {
    let mut key = remote.trim().to_owned();
    for prefix in ["https://", "http://", "ssh://", "git://"] {
        if let Some(rest) = key.clone().strip_prefix(prefix) {
            key = rest.to_owned();
        }
    }
    // ssh://git@host/path and git@host:path both become host/path.
    if let Some(rest) = key.strip_prefix("git@") {
        key = rest.to_owned();
    }
    key = key.replacen(':', "/", 1).replace('\\', "/");
    while key.ends_with('/') {
        key.pop();
    }
    if let Some(stripped) = key.strip_suffix(".git") {
        key = stripped.to_owned();
    }
    key.to_lowercase()
}

/// The last path segment, matched case-insensitively like KSG's
/// `normalizeProjectPathForComparison` on case-folding filesystems.
pub fn folder_name(path: &str) -> String {
    path.trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .to_lowercase()
}

/// One logical-project member on the target machine, or none. `candidates`
/// must be the target machine's projects; hidden entries are skipped.
/// `source` is the draft's current project on its own machine.
pub fn match_logical_project(
    source: &RouteProject,
    candidates: &[RouteProject],
) -> Option<usize> {
    let open = |p: &&RouteProject| !p.hidden;
    let winner = |projects: Vec<&RouteProject>| {
        projects
            .into_iter()
            .max_by_key(|p| (p.updated, p.id))
            .map(|p| p.id)
    };
    if let Some(remote) = source.git_remote.as_deref() {
        let key = canonical_remote(remote);
        winner(
            candidates
                .iter()
                .filter(open)
                .filter(|p| {
                    p.git_remote
                        .as_deref()
                        .is_some_and(|remote| canonical_remote(remote) == key)
                })
                .collect(),
        )
    } else {
        let folder = folder_name(&source.path);
        if folder.is_empty() {
            return None;
        }
        winner(
            candidates
                .iter()
                .filter(open)
                .filter(|p| p.git_remote.is_none())
                .filter(|p| folder_name(&p.path) == folder)
                .collect(),
        )
    }
}

/// What a machine change keeps or drops, per draft kind. Mirrors KSG's
/// `setDraftThreadContext`: the draft body is preserved wholesale while
/// checkout-specific state is reset on a project change.
#[derive(Default, Debug, PartialEq, Eq)]
pub struct RetargetResets {
    pub model: bool,
    pub worktree: bool,
}

impl RetargetResets {
    /// The single notification KSG-style toasts carry when the switch reset
    /// anything, listing what changed.
    pub fn notice(&self, machine_label: &str) -> Option<String> {
        let mut items = Vec::new();
        if self.model {
            items.push("model selection");
        }
        if self.worktree {
            items.push("worktree settings");
        }
        if items.is_empty() {
            return None;
        }
        Some(format!(
            "Switched to {machine_label}. Reset: {}.",
            items.join(", ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(id: usize, path: &str, remote: Option<&str>, updated: u64) -> RouteProject {
        RouteProject {
            id,
            path: path.into(),
            git_remote: remote.map(str::to_owned),
            hidden: false,
            updated,
        }
    }

    #[test]
    fn matches_the_same_repository_on_another_machine() {
        let source = project(3, "/Users/a/work/app", Some("git@github.com:team/app.git"), 10);
        let candidates = [
            project(1, "/home/b/code/other", Some("git@github.com:team/other.git"), 20),
            project(7, "/home/b/code/app", Some("https://github.com/team/app.git"), 15),
        ];
        assert_eq!(match_logical_project(&source, &candidates), Some(7));
    }

    #[test]
    fn matches_a_folder_name_when_neither_side_has_a_repository() {
        let source = project(3, "/Users/a/notes/planning", None, 10);
        let candidates = [
            project(1, "/srv/work/other", None, 20),
            project(9, "/srv/shared/planning", None, 5),
        ];
        assert_eq!(match_logical_project(&source, &candidates), Some(9));
    }

    #[test]
    fn a_repository_project_never_falls_back_to_folder_names() {
        let source = project(3, "/a/app", Some("git@github.com:team/app.git"), 10);
        let candidates = [project(4, "/b/app", None, 30)];
        assert_eq!(match_logical_project(&source, &candidates), None);
    }

    #[test]
    fn two_candidates_pick_the_freshest_then_the_higher_id() {
        let source = project(3, "/a/app", Some("git@github.com:team/app.git"), 10);
        let candidates = [
            project(4, "/b/app", Some("git@github.com:team/app.git"), 30),
            project(9, "/b/app2", Some("git@github.com:team/app.git"), 30),
            project(2, "/b/app3", Some("git@github.com:team/app.git"), 40),
        ];
        assert_eq!(match_logical_project(&source, &candidates), Some(2));
        // Equal freshness resolves to the higher id (KSG's winner rule).
        let candidates = [
            project(4, "/b/app", Some("git@github.com:team/app.git"), 30),
            project(9, "/b/app2", Some("git@github.com:team/app.git"), 30),
        ];
        assert_eq!(match_logical_project(&source, &candidates), Some(9));
    }

    #[test]
    fn no_match_when_the_target_has_nothing_alike() {
        let source = project(3, "/a/app", Some("git@github.com:team/app.git"), 10);
        let candidates = [
            project(1, "/b/other", Some("git@github.com:team/other.git"), 20),
            project(2, "/b/more", None, 20),
        ];
        assert_eq!(match_logical_project(&source, &candidates), None);
        // A hidden project is not a candidate.
        let mut hidden = project(5, "/b/app", Some("git@github.com:team/app.git"), 50);
        hidden.hidden = true;
        assert_eq!(match_logical_project(&source, &[hidden]), None);
    }

    #[test]
    fn resets_list_only_what_changed() {
        assert_eq!(RetargetResets::default().notice("Studio"), None);
        let resets = RetargetResets {
            model: true,
            worktree: true,
        };
        assert_eq!(
            resets.notice("Studio").as_deref(),
            Some("Switched to Studio. Reset: model selection, worktree settings.")
        );
    }
}
