//! Pure branch-toolbar and workspace-mode logic ported from KSG Code's
//! `components/BranchToolbar.logic.ts` and `packages/shared/src/git.ts`.

use std::collections::HashSet;

use crate::{
    model::ThreadEnvMode,
    vcs::{VcsRef, derive_local_branch_name_from_remote_ref, sanitize_new_ref_name},
};

pub fn resolve_env_mode_label(mode: ThreadEnvMode) -> &'static str {
    match mode {
        ThreadEnvMode::Worktree => "New worktree",
        ThreadEnvMode::Local => "Current checkout",
    }
}

#[allow(dead_code)]
pub fn resolve_current_workspace_label(active_worktree_path: Option<&str>) -> &'static str {
    if active_worktree_path.is_some() {
        "Current worktree"
    } else {
        resolve_env_mode_label(ThreadEnvMode::Local)
    }
}

pub fn resolve_locked_workspace_label(
    active_worktree_path: Option<&str>,
    effective_env_mode: ThreadEnvMode,
) -> String {
    if let Some(path) = active_worktree_path {
        return path
            .split(['/', '\\'])
            .rfind(|segment| !segment.is_empty())
            .unwrap_or("Worktree")
            .to_owned();
    }
    if effective_env_mode == ThreadEnvMode::Worktree {
        resolve_env_mode_label(ThreadEnvMode::Worktree).to_owned()
    } else {
        "Local checkout".to_owned()
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn resolve_effective_env_mode(
    active_worktree_path: Option<&str>,
    has_server_thread: bool,
    draft_thread_env_mode: Option<ThreadEnvMode>,
    preparing_worktree: bool,
) -> ThreadEnvMode {
    if !has_server_thread {
        if active_worktree_path.is_some() {
            return ThreadEnvMode::Local;
        }
        return match draft_thread_env_mode {
            Some(ThreadEnvMode::Worktree) => ThreadEnvMode::Worktree,
            _ => ThreadEnvMode::Local,
        };
    }
    if active_worktree_path.is_some() || preparing_worktree {
        ThreadEnvMode::Worktree
    } else {
        ThreadEnvMode::Local
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn resolve_draft_env_mode_after_branch_change(
    next_worktree_path: Option<&str>,
    current_worktree_path: Option<&str>,
    effective_env_mode: ThreadEnvMode,
) -> ThreadEnvMode {
    if next_worktree_path.is_some() {
        return ThreadEnvMode::Worktree;
    }
    if effective_env_mode == ThreadEnvMode::Worktree && current_worktree_path.is_none() {
        return ThreadEnvMode::Worktree;
    }
    ThreadEnvMode::Local
}

#[allow(dead_code)]
pub fn resolve_branch_toolbar_value(
    env_mode: ThreadEnvMode,
    active_worktree_path: Option<&str>,
    active_thread_branch: Option<&str>,
    current_git_branch: Option<&str>,
) -> Option<String> {
    if env_mode == ThreadEnvMode::Worktree && active_worktree_path.is_none() {
        return active_thread_branch
            .or(current_git_branch)
            .map(str::to_owned);
    }
    current_git_branch
        .or(active_thread_branch)
        .map(str::to_owned)
}

pub fn resolve_branch_trigger_label(
    active_worktree_path: Option<&str>,
    effective_env_mode: ThreadEnvMode,
    resolved_active_branch: Option<&str>,
    resolved_active_branch_is_remote: Option<bool>,
    start_from_origin: bool,
) -> String {
    let Some(branch) = resolved_active_branch.filter(|b| !b.trim().is_empty()) else {
        return "Select ref".to_owned();
    };
    if effective_env_mode == ThreadEnvMode::Worktree && active_worktree_path.is_none() {
        return if start_from_origin && resolved_active_branch_is_remote == Some(false) {
            format!("From {branch} (remote)")
        } else {
            format!("From {branch}")
        };
    }
    branch.to_owned()
}

/// Ported from KSG Code's `BranchToolbarBranchSelector.tsx` default base branch rule:
/// `defaultBranchName ?? currentGitBranch` once branches have loaded.
pub fn default_worktree_base_branch(
    refs: &[VcsRef],
    current_git_branch: Option<&str>,
) -> Option<String> {
    refs.iter()
        .find(|r| r.is_default)
        .map(|r| r.name.clone())
        .or_else(|| {
            current_git_branch
                .map(str::trim)
                .filter(|b| !b.is_empty())
                .map(str::to_owned)
        })
        .or_else(|| refs.iter().find(|r| r.current).map(|r| r.name.clone()))
}

pub fn should_include_branch_picker_item(
    item_value: &str,
    normalized_query: &str,
    create_branch_item_value: Option<&str>,
    checkout_pull_request_item_value: Option<&str>,
) -> bool {
    if normalized_query.is_empty() {
        return true;
    }
    if create_branch_item_value.is_some_and(|v| v == item_value) {
        return true;
    }
    if checkout_pull_request_item_value.is_some_and(|v| v == item_value) {
        return true;
    }
    let lower_item = item_value.to_ascii_lowercase();
    if lower_item.contains(normalized_query) {
        return true;
    }
    let sanitized_query = sanitize_new_ref_name(normalized_query);
    !sanitized_query.is_empty()
        && sanitized_query != normalized_query
        && lower_item.contains(&sanitized_query)
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn dedupe_remote_branches_with_local_matches(refs: Vec<VcsRef>) -> Vec<VcsRef> {
    let local_names: HashSet<String> = refs
        .iter()
        .filter(|r| r.is_remote != Some(true))
        .map(|r| r.name.clone())
        .collect();
    refs.into_iter()
        .filter(|r| {
            if r.is_remote == Some(true) && r.remote_name.as_deref() == Some("origin") {
                let local_name = derive_local_branch_name_from_remote_ref(&r.name);
                if !local_name.is_empty() && local_names.contains(local_name) {
                    return false;
                }
            }
            true
        })
        .collect()
}

/// Orders branches for the picker: local branches first, then remote branches,
/// keeping KSG's priority order (current, default, recency) within each group.
pub fn order_branches_for_picker(refs: &[VcsRef]) -> Vec<&VcsRef> {
    let mut local = Vec::new();
    let mut remote = Vec::new();
    for r in refs {
        if r.is_remote == Some(true) {
            remote.push(r);
        } else {
            local.push(r);
        }
    }
    local.extend(remote);
    local
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vcs_ref(name: &str) -> VcsRef {
        VcsRef {
            name: name.to_owned(),
            is_remote: None,
            remote_name: None,
            upstream_ref: None,
            current: false,
            is_default: false,
            worktree_path: None,
        }
    }

    #[test]
    fn resolve_draft_env_mode_after_branch_change_cases() {
        assert_eq!(
            resolve_draft_env_mode_after_branch_change(
                None,
                Some("/repo/.t3/worktrees/feature-a"),
                ThreadEnvMode::Worktree,
            ),
            ThreadEnvMode::Local
        );
        assert_eq!(
            resolve_draft_env_mode_after_branch_change(
                Some("/repo/.t3/worktrees/feature-a"),
                None,
                ThreadEnvMode::Local,
            ),
            ThreadEnvMode::Worktree
        );
    }

    #[test]
    fn resolve_effective_env_mode_cases() {
        assert_eq!(
            resolve_effective_env_mode(None, true, None, true),
            ThreadEnvMode::Worktree
        );
        assert_eq!(
            resolve_effective_env_mode(None, true, None, false),
            ThreadEnvMode::Local
        );
    }

    #[test]
    fn resolve_locked_workspace_label_cases() {
        for (path, expected) in [
            ("/home/me/.t3/worktrees/app/t3-ff466c42", "t3-ff466c42"),
            ("/home/me/.t3/worktrees/app/t3-ff466c42/", "t3-ff466c42"),
            ("C:\\Users\\me\\worktrees\\feature-x", "feature-x"),
        ] {
            assert_eq!(
                resolve_locked_workspace_label(Some(path), ThreadEnvMode::Worktree),
                expected
            );
        }
    }

    #[test]
    fn derive_local_branch_name_from_remote_ref_cases() {
        assert_eq!(
            derive_local_branch_name_from_remote_ref("my-org/upstream/feature/demo"),
            "upstream/feature/demo"
        );
    }

    #[test]
    fn dedupe_remote_branches_with_local_matches_cases() {
        for (local_name, remote_name, hidden) in [
            ("feature/demo", "origin/feature/demo", true),
            ("feature/demo", "feature/demo", false),
            ("demo", "demo", true),
            ("origin/", "origin/", true),
            ("/demo", "/demo", true),
            ("", "", false),
        ] {
            let mut local = vcs_ref(local_name);
            local.is_remote = None;
            let mut remote = vcs_ref(remote_name);
            remote.is_remote = Some(true);
            remote.remote_name = Some("origin".to_owned());
            let mut remote_only = vcs_ref("origin/feature/remote-only");
            remote_only.is_remote = Some(true);
            remote_only.remote_name = Some("origin".to_owned());

            let input = vec![local.clone(), remote.clone(), remote_only.clone()];
            let result = dedupe_remote_branches_with_local_matches(input.clone());
            let expected = if hidden {
                vec![local, remote_only]
            } else {
                input
            };
            assert_eq!(result, expected, "{local_name} / {remote_name}");
        }

        let mut upstream = vcs_ref("upstream/feature");
        upstream.is_remote = None;
        let mut remote_upstream = vcs_ref("my-org/upstream/feature");
        remote_upstream.is_remote = Some(true);
        remote_upstream.remote_name = Some("my-org/upstream".to_owned());
        let names: Vec<String> =
            dedupe_remote_branches_with_local_matches(vec![upstream, remote_upstream])
                .into_iter()
                .map(|r| r.name)
                .collect();
        assert_eq!(names, vec!["upstream/feature", "my-org/upstream/feature"]);
    }

    #[test]
    fn should_include_branch_picker_item_cases() {
        assert!(should_include_branch_picker_item(
            "__checkout_pull_request__:1359",
            "gh pr checkout 1359",
            Some("__create_new_branch__:gh pr checkout 1359"),
            Some("__checkout_pull_request__:1359"),
        ));
        assert!(should_include_branch_picker_item(
            "new-branch",
            "new branch",
            None,
            None,
        ));
    }

    #[test]
    fn sanitize_new_ref_name_cases() {
        assert_eq!(sanitize_new_ref_name("new   branch"), "new-branch");
        assert_eq!(sanitize_new_ref_name("new\tbranch"), "new-branch");
        assert_eq!(
            sanitize_new_ref_name("new\u{00a0}branch"),
            "new\u{00a0}branch"
        );
        assert_eq!(
            sanitize_new_ref_name("new\u{2009}branch"),
            "new\u{2009}branch"
        );
        assert_eq!(sanitize_new_ref_name("new - branch"), "new---branch");
        assert_eq!(sanitize_new_ref_name("foo--bar"), "foo--bar");
    }

    #[test]
    fn default_worktree_base_branch_prefers_default_over_current() {
        let mut current = vcs_ref("feature/wip");
        current.current = true;
        let mut main = vcs_ref("main");
        main.is_default = true;
        assert_eq!(
            default_worktree_base_branch(&[current.clone(), main], Some("feature/wip")).as_deref(),
            Some("main")
        );
        assert_eq!(
            default_worktree_base_branch(&[current], None).as_deref(),
            Some("feature/wip")
        );
    }

    #[test]
    fn resolve_branch_trigger_label_cases() {
        assert_eq!(
            resolve_branch_trigger_label(None, ThreadEnvMode::Worktree, None, None, true),
            "Select ref"
        );
        assert_eq!(
            resolve_branch_trigger_label(
                None,
                ThreadEnvMode::Worktree,
                Some("main"),
                Some(false),
                true,
            ),
            "From main (remote)"
        );
        assert_eq!(
            resolve_branch_trigger_label(
                None,
                ThreadEnvMode::Worktree,
                Some("main"),
                Some(false),
                false,
            ),
            "From main"
        );
        assert_eq!(
            resolve_branch_trigger_label(
                Some("/tmp/wt"),
                ThreadEnvMode::Worktree,
                Some("t3/deadbeef"),
                Some(false),
                true,
            ),
            "t3/deadbeef"
        );
    }
}
