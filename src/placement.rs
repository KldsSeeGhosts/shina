//! The placement contract shared by delegated tasks and independent crew.
//!
//! Every worker gets an explicit placement: a shared read-only view of the
//! project's own checkout, a new isolated Git worktree, or an explicitly
//! selected existing worktree. Writers default to an isolated worktree; the
//! caller's prompt text never decides reader versus writer. The reservation
//! (repository, base, branch and checkout) is recorded durably before any
//! filesystem provisioning, so a same-key retry or a restart reuses the exact
//! worktree instead of allocating another.
use super::*;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlacementMode {
    /// A read-only view of the project's own checkout. The worker sees the
    /// parent's dirty working tree and cannot write to it.
    Shared,
    /// A new Git worktree owned by this worker, on its own branch.
    Isolated,
    /// An explicitly selected, already-registered worktree of the repository.
    Existing,
}

/// The mode the caller asked for. `Default` means the writer default, which is
/// an isolated worktree when the workspace supports one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlacementRequest {
    Default,
    Shared,
    Isolated,
    Existing,
}
impl PlacementRequest {
    pub(crate) fn from_mode(mode: Option<PlacementMode>) -> Self {
        match mode {
            None => Self::Default,
            Some(PlacementMode::Shared) => Self::Shared,
            Some(PlacementMode::Isolated) => Self::Isolated,
            Some(PlacementMode::Existing) => Self::Existing,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlacementPhase {
    /// The reservation is durable; provisioning has not finished.
    Pending,
    /// The checkout is provisioned and provider work may start.
    Ready,
    /// Provisioning or setup failed before any provider started.
    Failed,
}

/// `mode` accepts the documented `"default"` as an explicit writer default.
fn spec_mode<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<PlacementMode>, D::Error> {
    match Option::<Value>::deserialize(d)? {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(mode)) if mode == "default" => Ok(None),
        Some(value) => PlacementMode::deserialize(value)
            .map(Some)
            .map_err(serde::de::Error::custom),
    }
}

/// The caller's placement selection inside `delegate_task`, `captain_crew_create`
/// or `peer.thread.launch` arguments.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlacementSpec {
    #[serde(default, deserialize_with = "spec_mode")]
    pub mode: Option<PlacementMode>,
    #[serde(default)]
    pub base_branch: Option<String>,
    #[serde(default)]
    pub base_commit: Option<String>,
    #[serde(default)]
    pub upstream_thread_id: Option<usize>,
    #[serde(default)]
    pub start_from_origin: Option<bool>,
    /// Explicit name for the new worktree branch.
    #[serde(default)]
    pub branch: Option<String>,
    /// Existing-worktree selector: an absolute path or a branch name.
    #[serde(default)]
    pub worktree: Option<String>,
}

/// Checkout-binding provisioning state on the executor. `pending` means the
/// executor is still materializing the clone; `ready` means an executor-local
/// checkout serves the placement; `failed` keeps the recorded reason for a
/// retry that reuses the same target directory.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BindingPhase {
    Pending,
    Ready,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingFailure {
    CredentialsUnavailable,
    ProvisioningFailed,
}

impl BindingFailure {
    pub(super) fn from_error(error: &str) -> Self {
        let error = error.to_ascii_lowercase();
        if [
            "authentication failed",
            "could not read username",
            "could not read password",
            "terminal prompts disabled",
            "permission denied (publickey",
            "http 401",
            "http 403",
            "requested url returned error: 401",
            "requested url returned error: 403",
        ]
        .iter()
        .any(|message| error.contains(message))
        {
            Self::CredentialsUnavailable
        } else {
            Self::ProvisioningFailed
        }
    }
}

/// The logical-repository binding a placement resolved or is still resolving.
/// `spec` holds the caller's deferred placement selection while the executor
/// provisions the checkout; it is cleared once planning runs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlacementBinding {
    /// The controller's logical repository id (`repo-…`).
    pub repository: String,
    /// Canonical remotes the binding was resolved against.
    #[serde(default)]
    pub remotes: Vec<String>,
    /// The clone source the executor used or will use.
    #[serde(default)]
    pub clone_url: String,
    /// The executor-local checkout that serves, or will serve, the repository.
    #[serde(default)]
    pub checkout: String,
    pub state: BindingPhase,
    #[serde(default)]
    pub spec: Option<PlacementSpec>,
    /// Distinct failure reason: missing clone credentials, a non-Git target
    /// directory, an ambiguous binding, or a failed clone.
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<BindingFailure>,
}

/// The durable reservation and resolved placement of one worker thread. It is
/// recorded on the thread before any filesystem work starts, projected to
/// clients in the thread record, returned by `task_status` and carried back to
/// a delegating peer in `peer.thread.wait` statuses.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadPlacement {
    /// What the caller asked for: `default` is the writer default.
    pub requested: PlacementRequest,
    /// The negotiated outcome.
    pub mode: PlacementMode,
    pub phase: PlacementPhase,
    /// The machine that runs the checkout: `local` or a peer server id.
    #[serde(default)]
    pub executor: String,
    /// The repository root the placement belongs to, when one exists.
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub base_branch: Option<String>,
    /// The commit the new worktree started from, once resolved on the executor.
    #[serde(default)]
    pub base_commit: Option<String>,
    #[serde(default)]
    pub upstream_thread_id: Option<usize>,
    /// The branch checked out for isolated or existing placements.
    #[serde(default)]
    pub branch: Option<String>,
    /// The directory the provider actually runs in.
    #[serde(default)]
    pub checkout: Option<String>,
    #[serde(default)]
    pub start_from_origin: bool,
    /// The selected checkout is the repository's primary working tree.
    #[serde(default)]
    pub primary_checkout: bool,
    /// Shina created (or will create) this worktree for this worker.
    #[serde(default)]
    pub new_worktree: bool,
    /// Why the requested isolation is unavailable, when it was downgraded.
    #[serde(default)]
    pub capability: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    /// The logical-repository binding this placement runs on, when delegation
    /// named a catalog repository instead of a machine-local project.
    #[serde(default)]
    pub binding: Option<PlacementBinding>,
}

impl ThreadPlacement {
    /// A shared placement never grants write access to a real project
    /// checkout. A private scratch directory stays unrestricted.
    pub fn read_only(&self) -> bool {
        self.mode == PlacementMode::Shared && (self.primary_checkout || self.repository.is_some())
    }
    pub fn uses_worktree(&self) -> bool {
        self.mode == PlacementMode::Existing
            || self.mode == PlacementMode::Shared
                && !self.primary_checkout
                && self.repository.is_some()
    }
    pub fn value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

pub(crate) struct PlacementJobOutcome {
    pub thread_id: usize,
    pub result: Result<ProvisionedPlacement, String>,
}

pub(crate) struct ProvisionedPlacement {
    pub worktree_path: String,
    pub branch: String,
    /// The resolved commit the worktree starts from.
    pub head: String,
    pub base_branch: Option<String>,
}

/// Parse the `placement` argument. Absent or null means the caller left the
/// writer default.
pub fn spec_from_args(args: &Value) -> Result<Option<PlacementSpec>, String> {
    match args.get("placement") {
        None | Some(Value::Null) => Ok(None),
        Some(value) => serde_json::from_value::<PlacementSpec>(value.clone())
            .map(Some)
            .map_err(|error| {
                format!(
                    "placement must be an object with mode shared, isolated or existing: {error}"
                )
            }),
    }
}

fn nonempty(value: Option<&String>) -> Option<&str> {
    value.map(|s| s.trim()).filter(|s| !s.is_empty())
}

/// The mode the caller is treated as having asked for, plus field validation.
/// Selecting a worktree or a base implies an explicit mode even when `mode`
/// was omitted.
pub fn requested_mode(spec: &PlacementSpec) -> Option<PlacementMode> {
    spec.mode.or_else(|| {
        if spec.worktree.is_some() {
            Some(PlacementMode::Existing)
        } else if spec.base_branch.is_some()
            || spec.base_commit.is_some()
            || spec.upstream_thread_id.is_some()
            || spec.start_from_origin.is_some()
            || spec.branch.is_some()
        {
            Some(PlacementMode::Isolated)
        } else {
            None
        }
    })
}

pub fn validate_spec(spec: &PlacementSpec) -> Result<Option<PlacementMode>, String> {
    let requested = requested_mode(spec);
    for (name, value) in [
        ("baseBranch", &spec.base_branch),
        ("baseCommit", &spec.base_commit),
        ("branch", &spec.branch),
        ("worktree", &spec.worktree),
    ] {
        if value.as_deref().is_some_and(|v| v.trim().is_empty()) {
            return Err(format!("placement.{name} must be non-empty."));
        }
    }
    match requested {
        Some(PlacementMode::Shared) => {
            if spec.base_branch.is_some()
                || spec.base_commit.is_some()
                || spec.upstream_thread_id.is_some()
                || spec.start_from_origin.is_some()
                || spec.branch.is_some()
                || spec.worktree.is_some()
            {
                return Err("A shared placement takes no base, branch or worktree fields.".into());
            }
        }
        Some(PlacementMode::Existing) => {
            if spec.worktree.is_none() {
                return Err(
                    "An existing placement needs placement.worktree naming a path or branch."
                        .into(),
                );
            }
            if spec.base_branch.is_some()
                || spec.base_commit.is_some()
                || spec.upstream_thread_id.is_some()
                || spec.start_from_origin.is_some()
                || spec.branch.is_some()
            {
                return Err(
                    "An existing placement selects a worktree only; base and branch fields do not apply."
                        .into(),
                );
            }
        }
        _ => {
            if spec.worktree.is_some() {
                return Err(
                    "placement.worktree selects an existing worktree; use mode existing.".into(),
                );
            }
            let bases = spec.base_branch.is_some() as usize
                + spec.base_commit.is_some() as usize
                + spec.upstream_thread_id.is_some() as usize;
            if bases > 1 {
                return Err(
                    "An isolated placement takes one base: baseBranch, baseCommit or upstreamThreadId."
                        .into(),
                );
            }
            if spec.start_from_origin == Some(true)
                && (spec.base_commit.is_some() || spec.upstream_thread_id.is_some())
            {
                return Err("placement.startFromOrigin only applies to a base branch.".into());
            }
        }
    }
    Ok(requested)
}

/// The branch a placement records for itself. The name derives from the
/// reservation seed, so a retry or restart reuses the exact branch.
pub fn planned_branch_name(seed: &str, explicit: Option<&str>) -> String {
    if let Some(explicit) = explicit {
        return explicit.trim().to_owned();
    }
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(format!("shina.placement.branch:{seed}").as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("{}/{}", crate::vcs::WORKTREE_BRANCH_PREFIX, &hex[..16])
}

pub(crate) enum PlannedBase {
    /// The repository's default branch, or its current branch as a fallback.
    DefaultBranch,
    Branch(String),
    Commit(String),
    UpstreamBranch(String),
}

pub(crate) struct IsolatedPlan {
    repo: PathBuf,
    checkout: PathBuf,
    branch: String,
    base: PlannedBase,
    base_branch: Option<String>,
    start_from_origin: bool,
    worktrees_dir: PathBuf,
}

/// Resolve the caller's spec against this executor's workspace. Shared and
/// existing placements resolve synchronously because they need no filesystem
/// writes. An isolated placement returns a pending reservation whose planned
/// branch and checkout are recorded before `git worktree add` ever runs.
impl State {
    fn scratch_checkout(&self, thread: &Thread) -> PathBuf {
        thread
            .scratch_directory
            .as_deref()
            .map(PathBuf::from)
            .unwrap_or_else(|| self.data_dir.join("chats").join(thread.id.to_string()))
    }

    fn shared_checkout_info(
        &self,
        cwd: &Path,
        ctx: &crate::vcs::GitContext,
    ) -> (Option<String>, Option<String>) {
        let branch = crate::vcs::run_git_raw(
            "Placement.sharedBranch",
            cwd,
            &["symbolic-ref", "--short", "HEAD"],
            crate::vcs::SHORT_TIMEOUT,
            ctx,
        )
        .ok()
        .filter(|out| out.exit_code == Some(0))
        .map(|out| out.stdout.trim().to_owned())
        .filter(|s| !s.is_empty());
        let head = crate::vcs::run_git_raw(
            "Placement.sharedHead",
            cwd,
            &["rev-parse", "HEAD"],
            crate::vcs::SHORT_TIMEOUT,
            ctx,
        )
        .ok()
        .filter(|out| out.exit_code == Some(0))
        .map(|out| out.stdout.trim().to_owned())
        .filter(|s| !s.is_empty());
        (branch, head)
    }

    /// Resolve a placement spec for a thread that has not been pushed yet.
    /// Validation failures reject the caller's intent before any task, thread
    /// or filesystem work exists.
    pub(super) fn plan_thread_placement(
        &self,
        thread: &Thread,
        spec: Option<&PlacementSpec>,
        seed: &str,
        executor: &str,
        ctx: &crate::vcs::GitContext,
    ) -> Result<ThreadPlacement, String> {
        let spec = spec.cloned().unwrap_or_default();
        let requested = validate_spec(&spec)?;
        let project = self.project(thread.project);
        let project_path = project.map(|p| p.path.clone());
        // A reader of a local parent's tree must see that parent's checkout,
        // including uncommitted changes in an existing worktree.
        let shared_worktree = if thread.peer_origin.is_none() {
            thread
                .parent
                .or_else(|| thread.crew_captain.and_then(|id| self.captain_home(id)))
                .and_then(|parent| self.threads.iter().find(|t| t.id == parent))
                .filter(|parent| {
                    parent.project == thread.project && parent.machine == thread.machine
                })
                .and_then(|parent| parent.worktree_path.as_deref())
                .map(PathBuf::from)
        } else {
            None
        };
        let mut git_error = None;
        let common_dir = match project_path
            .as_deref()
            .map(|cwd| crate::vcs::read_git_common_dir(cwd, ctx))
        {
            Some(Ok(dir)) => dir,
            Some(Err(error)) => {
                git_error = Some(error);
                None
            }
            None => None,
        };
        let is_repo = common_dir.is_some();
        let shared_worktree = shared_worktree.filter(|_| is_repo);
        let mut record = ThreadPlacement {
            requested: PlacementRequest::from_mode(requested),
            mode: PlacementMode::Shared,
            phase: PlacementPhase::Ready,
            executor: executor.into(),
            repository: None,
            base_branch: None,
            base_commit: None,
            upstream_thread_id: None,
            branch: None,
            checkout: None,
            start_from_origin: spec.start_from_origin.unwrap_or(false),
            primary_checkout: false,
            new_worktree: false,
            capability: None,
            error: None,
            binding: None,
        };
        let shared = |record: &mut ThreadPlacement, capability: Option<String>| {
            record.capability = capability;
            match &project_path {
                Some(path) => {
                    let checkout = shared_worktree.as_ref().unwrap_or(path);
                    record.checkout = Some(checkout.to_string_lossy().into_owned());
                    record.primary_checkout = shared_worktree.is_none();
                    if is_repo {
                        record.repository = Some(path.to_string_lossy().into_owned());
                        let (branch, head) = self.shared_checkout_info(checkout, ctx);
                        record.branch = branch;
                        record.base_commit = head;
                    }
                }
                None => {
                    record.checkout =
                        Some(self.scratch_checkout(thread).to_string_lossy().into_owned());
                    if record.capability.is_none() {
                        record.capability = Some(
                            "This thread has no project workspace; it runs in a private scratch directory, not a Git worktree."
                                .into(),
                        );
                    }
                }
            }
        };
        match requested.unwrap_or(PlacementMode::Isolated) {
            PlacementMode::Shared => {
                shared(&mut record, None);
            }
            PlacementMode::Isolated => {
                let Some(path) = &project_path else {
                    if requested.is_some() {
                        return Err(
                            "An isolated worktree needs a project workspace; this thread has none. Use mode shared for the scratch directory."
                                .into(),
                        );
                    }
                    shared(&mut record, None);
                    return Ok(record);
                };
                if !is_repo {
                    let name = project.map(|p| p.name.as_str()).unwrap_or("workspace");
                    if requested.is_some() {
                        return Err(format!(
                            "Project {name} is not a Git repository{}; it cannot host an isolated worktree. Use mode shared for the project checkout.",
                            git_error.map(|e| format!(" ({e})")).unwrap_or_default()
                        ));
                    }
                    shared(
                        &mut record,
                        Some(format!(
                            "Project {name} is not a Git repository{}; the worker shares its checkout instead.",
                            git_error.map(|e| format!(" ({e})")).unwrap_or_default()
                        )),
                    );
                    return Ok(record);
                }
                if self.demo {
                    // Demo states simulate providers and cannot run real
                    // worktree provisioning; keep the shared checkout.
                    if requested.is_some() {
                        return Err("Demo states cannot provision isolated worktrees.".into());
                    }
                    shared(
                        &mut record,
                        Some("Demo states do not provision worktrees; the worker shares the checkout.".into()),
                    );
                    return Ok(record);
                }
                let has_commits = crate::vcs::run_git_raw(
                    "Placement.headCommit",
                    path,
                    &["rev-parse", "--verify", "--quiet", "HEAD^{commit}"],
                    crate::vcs::SHORT_TIMEOUT,
                    ctx,
                )
                .map(|out| out.exit_code == Some(0))
                .unwrap_or(false);
                if !has_commits {
                    let name = project.map(|p| p.name.as_str()).unwrap_or("workspace");
                    if requested.is_some() {
                        return Err(format!(
                            "Project {name} has no commits yet; an isolated worktree needs a base commit. Use mode shared for the project checkout."
                        ));
                    }
                    shared(
                        &mut record,
                        Some(format!(
                            "Project {name} has no commits yet, so it cannot host a worktree; the worker shares its checkout instead."
                        )),
                    );
                    return Ok(record);
                }
                let upstream = match spec.upstream_thread_id {
                    None => None,
                    Some(id) => {
                        if id == thread.id {
                            return Err(
                                "placement.upstreamThreadId must name a different thread.".into()
                            );
                        }
                        let upstream = self
                            .threads
                            .iter()
                            .find(|t| t.id == id)
                            .ok_or("placement.upstreamThreadId names an unknown thread.")?;
                        let upstream_placement = upstream.placement.as_ref().ok_or(
                            "placement.upstreamThreadId names a thread with no placement.",
                        )?;
                        if upstream_placement.phase != PlacementPhase::Ready
                            || upstream_placement.branch.is_none()
                        {
                            return Err(
                                "placement.upstreamThreadId names a thread whose worktree branch is not ready yet."
                                    .into(),
                            );
                        }
                        if upstream_placement.repository.as_deref()
                            != Some(path.to_string_lossy().as_ref())
                        {
                            return Err(
                                "placement.upstreamThreadId names a thread in a different repository."
                                    .into(),
                            );
                        }
                        upstream_placement.branch.clone().map(|branch| (id, branch))
                    }
                };
                let branch = planned_branch_name(seed, nonempty(spec.branch.as_ref()));
                crate::vcs::run_git_checked(
                    "Placement.branchName",
                    path,
                    &["check-ref-format", "--branch", &branch],
                    crate::vcs::SHORT_TIMEOUT,
                    "placement.branch must be a valid Git branch name.",
                    ctx,
                )?;
                if crate::vcs::branch_exists(path, &branch, ctx) {
                    return Err("The new placement branch already exists. Choose another branch, or use an existing placement.".into());
                }
                let common = common_dir.expect("repo checked above");
                let checkout = crate::vcs::build_worktree_path(
                    &self.data_dir.join("worktrees"),
                    path,
                    &common,
                    &branch,
                )?;
                record.mode = PlacementMode::Isolated;
                record.phase = PlacementPhase::Pending;
                record.repository = Some(path.to_string_lossy().into_owned());
                record.branch = Some(branch);
                record.checkout = Some(checkout.to_string_lossy().into_owned());
                record.new_worktree = true;
                if let Some((id, upstream_branch)) = upstream {
                    record.upstream_thread_id = Some(id);
                    record.base_branch = Some(upstream_branch);
                } else if let Some(base) = nonempty(spec.base_branch.as_ref()) {
                    record.base_branch = Some(base.to_owned());
                }
                if let Some(commit) = nonempty(spec.base_commit.as_ref()) {
                    record.base_commit = Some(commit.to_owned());
                }
                if record.base_branch.is_none() && record.base_commit.is_none() {
                    record.base_branch = default_base_branch(path, ctx)?;
                }
                let base = record
                    .base_commit
                    .as_deref()
                    .or(record.base_branch.as_deref())
                    .ok_or("Could not resolve a default base branch. Pass placement.baseBranch.")?;
                let resolved = crate::vcs::run_git_checked(
                    "Placement.baseCommit",
                    path,
                    &[
                        "rev-parse",
                        "--verify",
                        "--quiet",
                        "--end-of-options",
                        &format!("{base}^{{commit}}"),
                    ],
                    crate::vcs::SHORT_TIMEOUT,
                    "The placement base must name a commit in this repository.",
                    ctx,
                )?;
                record.base_commit = Some(resolved.stdout.trim().to_owned());
            }
            PlacementMode::Existing => {
                let selector = nonempty(spec.worktree.as_ref()).expect("validated above");
                let Some(path) = &project_path else {
                    return Err(
                        "An existing worktree needs a project workspace; this thread has none."
                            .into(),
                    );
                };
                if !is_repo {
                    return Err(format!(
                        "Project {} is not a Git repository; it has no worktrees to select.",
                        project.map(|p| p.name.as_str()).unwrap_or("workspace")
                    ));
                }
                let listed = crate::vcs::list_worktrees_with(
                    &crate::vcs::VcsListWorktreesInput {
                        cwd: path.to_string_lossy().into_owned(),
                    },
                    ctx,
                )?;
                let selector_path = Path::new(selector);
                let entry = listed
                    .worktrees
                    .iter()
                    .find(|entry| {
                        (selector_path.is_absolute()
                            && crate::vcs::real_location(selector_path)
                                == crate::vcs::real_location(Path::new(&entry.path)))
                            || entry.ref_name.as_deref() == Some(selector)
                            || entry.ref_name.as_deref()
                                == Some(format!("refs/heads/{selector}").as_str())
                    })
                    .ok_or_else(|| {
                        format!(
                            "No worktree at '{selector}' or branch '{selector}' in this repository. Pick one from the worktree list."
                        )
                    })?;
                if entry.locked {
                    return Err(format!(
                        "Worktree {} is locked and cannot be selected.",
                        entry.path
                    ));
                }
                let entry_path = Path::new(&entry.path);
                let primary = listed.worktrees.first().is_some_and(|first| {
                    crate::vcs::real_location(Path::new(&first.path))
                        == crate::vcs::real_location(entry_path)
                });
                record.mode = PlacementMode::Existing;
                record.repository = Some(path.to_string_lossy().into_owned());
                record.checkout = Some(entry.path.clone());
                record.branch = entry
                    .ref_name
                    .as_deref()
                    .map(|name| name.strip_prefix("refs/heads/").unwrap_or(name).to_owned());
                record.base_commit = entry.head.clone();
                record.primary_checkout = primary;
            }
        }
        Ok(record)
    }

    fn placement_provision_job(&self, index: usize) -> Option<IsolatedPlan> {
        // Setup owns this pending placement until its runner finishes. Starting
        // another adoption job here would launch the setup script again.
        if self.threads[index]
            .worktree_setup
            .as_ref()
            .is_some_and(|setup| setup.phase == WorktreeSetupPhase::Running)
        {
            return None;
        }
        let placement = self.threads[index].placement.as_ref()?;
        if placement.phase != PlacementPhase::Pending
            || placement.mode != PlacementMode::Isolated
            || !placement.new_worktree
        {
            return None;
        }
        let base = if !placement.start_from_origin
            && let Some(commit) = &placement.base_commit
        {
            PlannedBase::Commit(commit.clone())
        } else if let Some(branch) = &placement.base_branch {
            if placement.upstream_thread_id.is_some() {
                PlannedBase::UpstreamBranch(branch.clone())
            } else {
                PlannedBase::Branch(branch.clone())
            }
        } else if let Some(commit) = &placement.base_commit {
            PlannedBase::Commit(commit.clone())
        } else {
            PlannedBase::DefaultBranch
        };
        Some(IsolatedPlan {
            repo: PathBuf::from(placement.repository.as_ref()?),
            checkout: PathBuf::from(placement.checkout.as_ref()?),
            branch: placement.branch.clone()?,
            base,
            base_branch: placement.base_branch.clone(),
            start_from_origin: placement.start_from_origin,
            worktrees_dir: self.data_dir.join("worktrees"),
        })
    }

    /// A Pending placement on a thread whose work is still wanted gets
    /// provisioned. Delegated children want work while their task is queued;
    /// peer workers while no result is recorded; crew roots always.
    pub(super) fn placement_wanted(&self, thread: &Thread) -> bool {
        if thread.archived() {
            return false;
        }
        if thread.peer_origin.is_some() {
            return thread.task_result.is_none()
                && thread
                    .peer_origin
                    .as_ref()
                    .is_some_and(|origin| origin.result.is_none());
        }
        if thread.delegated {
            return self.tasks.iter().any(|task| {
                task.child_thread_id == thread.id && task.state == tasks::TaskState::Queued
            });
        }
        true
    }

    /// Drive pending placements: adopt or create the recorded worktree, then
    /// run the ordinary worktree setup script before any provider starts.
    /// Only the owning server provisions; a client projects remote placements
    /// without touching local Git.
    pub(super) fn poll_placements(&mut self) -> bool {
        if self.client.is_some() {
            return false;
        }
        let mut changed = false;
        let mut remaining = Vec::with_capacity(self.placement_rxs.len());
        let rxs = std::mem::take(&mut self.placement_rxs);
        for (thread_id, rx) in rxs {
            match rx.try_recv() {
                Ok(result) => {
                    changed = true;
                    self.apply_placement_outcome(PlacementJobOutcome { thread_id, result });
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => remaining.push((thread_id, rx)),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    changed = true;
                    self.apply_placement_outcome(PlacementJobOutcome {
                        thread_id,
                        result: Err("Worktree provisioning stopped unexpectedly.".into()),
                    });
                }
            }
        }
        self.placement_rxs = remaining;
        // Provision one reservation per tick. Concurrent `git worktree add`
        // calls in one repository collide on Git's own locks; serialization
        // keeps adoption checks and creation atomic in order.
        for index in 0..self.threads.len() {
            let thread_id = self.threads[index].id;
            if self.placement_rxs.iter().any(|(id, _)| *id == thread_id) {
                continue;
            }
            let Some(job) = self.placement_provision_job(index) else {
                continue;
            };
            if !self.placement_wanted(&self.threads[index]) {
                continue;
            }
            if !self.placement_rxs.is_empty() {
                break;
            }
            changed = true;
            #[cfg(test)]
            let git_ctx = self.test_git_context.clone().unwrap_or_default();
            #[cfg(not(test))]
            let git_ctx = crate::vcs::GitContext::default();
            let wake = self.wake.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            self.placement_rxs.push((thread_id, rx));
            std::thread::spawn(move || {
                let _ = tx.send(provision_isolated(&job, &git_ctx));
                wake();
            });
        }
        if changed {
            self.persistence_scope = None;
        }
        changed
    }

    fn apply_placement_outcome(&mut self, msg: PlacementJobOutcome) {
        let Some(index) = self
            .threads
            .iter()
            .position(|thread| thread.id == msg.thread_id)
        else {
            return;
        };
        match msg.result {
            Ok(provisioned) => {
                let wt_path = PathBuf::from(&provisioned.worktree_path);
                let project_cwd = self
                    .project(self.threads[index].project)
                    .map(|p| p.path.clone());
                let script_check = project_cwd
                    .as_deref()
                    .map(|cwd| crate::vcs::find_setup_script_info(&wt_path, cwd));
                {
                    let thread = &mut self.threads[index];
                    thread.worktree_path = Some(provisioned.worktree_path.clone());
                    thread.branch = Some(provisioned.branch.clone());
                    if let Some(base_branch) = &provisioned.base_branch {
                        thread.base_branch = Some(base_branch.clone());
                    }
                    thread.env_mode = Some(ThreadEnvMode::Worktree);
                    if let Some(placement) = thread.placement.as_mut() {
                        placement.checkout = Some(provisioned.worktree_path.clone());
                        placement.branch = Some(provisioned.branch.clone());
                        placement.base_commit = Some(provisioned.head.clone());
                        if let Some(base_branch) = provisioned.base_branch.clone() {
                            placement.base_branch = Some(base_branch);
                        }
                    }
                }
                match script_check {
                    Some(Ok(Some((script_name, script_command)))) => {
                        // A cancelled reservation can retain its checkout,
                        // but must not start a new setup process. A later
                        // explicit work request will adopt it and run setup.
                        if !self.placement_wanted(&self.threads[index]) {
                            return;
                        }
                        let thread = &mut self.threads[index];
                        thread.worktree_setup = Some(WorktreeSetupState {
                            phase: WorktreeSetupPhase::Running,
                            script_name: Some(script_name),
                            script_command: Some(script_command),
                            error: None,
                            output: String::new(),
                            exit_code: None,
                            timed_out: false,
                            duration_ms: 0,
                            skipped: false,
                        });
                        thread.worktree_setup_output_open = false;
                        let tid = thread.id;
                        self.spawn_worktree_setup_runner(tid);
                    }
                    Some(Err(error)) => {
                        self.fail_placement(index, error);
                    }
                    _ => {
                        self.finish_placement_ready(index);
                    }
                }
            }
            Err(error) => self.fail_placement(index, error),
        }
    }

    /// The checkout is provisioned and setup has finished; the worker's
    /// provider may now start through its ordinary route.
    pub(super) fn finish_placement_ready(&mut self, index: usize) {
        if let Some(placement) = self.threads[index].placement.as_mut() {
            placement.phase = PlacementPhase::Ready;
        }
        // A stop may have committed while Git or setup was running. Completing
        // the reservation must not revive that cancelled work.
        if !self.placement_wanted(&self.threads[index]) {
            return;
        }
        let thread = &self.threads[index];
        if thread.peer_origin.is_some() {
            let intents: Vec<_> = self
                .peer_task_intents
                .iter()
                .filter(|intent| intent.thread_id == thread.id && intent.kind == "input")
                .cloned()
                .collect();
            for intent in intents {
                self.provision_peer_task_intent(&intent);
            }
            if !self.threads[index].pending_injections.is_empty() {
                self.connect_thread(index);
            }
            return;
        }
        if thread.delegated {
            // The task gate opens; start_queued_tasks injects the brief and
            // connects through the ordinary route.
            self.start_queued_tasks();
            return;
        }
        self.release_after_worktree_setup(index);
    }

    /// Provisioning or setup failed before a provider ever started. The task
    /// or worker keeps its reservation and the failure; a retry reuses it.
    pub(super) fn fail_placement(&mut self, index: usize, error: String) {
        if let Some(placement) = self.threads[index].placement.as_mut() {
            placement.phase = PlacementPhase::Failed;
            placement.error = Some(error.clone());
        }
        self.threads[index].fail_session(error);
    }

    /// A new work attempt reuses the recorded reservation and provisions
    /// again instead of allocating a second checkout. A failed checkout
    /// binding retries its clone into the same recorded target first.
    pub(super) fn request_reprovision(&mut self, index: usize) {
        let mut retry_binding = None;
        if let Some(placement) = self.threads[index].placement.as_mut() {
            if let Some(binding) = placement.binding.as_mut()
                && binding.state == BindingPhase::Failed
            {
                binding.state = BindingPhase::Pending;
                binding.error = None;
                binding.error_code = None;
                retry_binding = Some((binding.repository.clone(), binding.checkout.clone()));
            }
            if placement.phase == PlacementPhase::Failed {
                placement.phase = PlacementPhase::Pending;
                placement.error = None;
            }
        }
        if let Some((repository, checkout)) = retry_binding
            && self.repository_bindings.iter().any(|binding| {
                binding.repository == repository
                    && binding.machine == "local"
                    && binding.path == checkout
                    && binding.state == repositories::BindingState::Failed
            })
        {
            self.record_binding(
                &repository,
                "local",
                None,
                &checkout,
                repositories::BindingState::Provisioning,
                Some("Retrying checkout provisioning on this executor.".into()),
            );
        }
    }
}

fn worktree_head(cwd: &Path, ctx: &crate::vcs::GitContext) -> Result<String, String> {
    let out = crate::vcs::run_git_checked(
        "Placement.worktreeHead",
        cwd,
        &["rev-parse", "HEAD"],
        crate::vcs::SHORT_TIMEOUT,
        "git rev-parse failed",
        ctx,
    )?;
    Ok(out.stdout.trim().to_owned())
}

fn default_base_branch(
    repo: &Path,
    ctx: &crate::vcs::GitContext,
) -> Result<Option<String>, String> {
    let listed = crate::vcs::list_branches(&crate::vcs::VcsListBranchesInput {
        cwd: repo.to_string_lossy().into_owned(),
        query: None,
        cursor: None,
        include_matching_remote_refs: None,
        ref_kind: Some(crate::vcs::VcsRefKind::Local),
        refresh: None,
        limit: Some(crate::vcs::GIT_LIST_BRANCHES_MAX_LIMIT),
    });
    let listed = match listed {
        Ok(listed) => listed,
        Err(_) => crate::vcs::list_branches(&crate::vcs::VcsListBranchesInput {
            cwd: repo.to_string_lossy().into_owned(),
            query: None,
            cursor: None,
            include_matching_remote_refs: None,
            ref_kind: None,
            refresh: None,
            limit: Some(crate::vcs::GIT_LIST_BRANCHES_MAX_LIMIT),
        })?,
    };
    let current = crate::vcs::run_git_raw(
        "Placement.currentBranch",
        repo,
        &["symbolic-ref", "--short", "HEAD"],
        crate::vcs::SHORT_TIMEOUT,
        ctx,
    )
    .ok()
    .filter(|out| out.exit_code == Some(0))
    .map(|out| out.stdout.trim().to_owned());
    Ok(crate::branch_toolbar::default_worktree_base_branch(
        &listed.refs,
        current.as_deref(),
    ))
}

/// Create or adopt the recorded worktree reservation. Adoption reuses the
/// exact branch and checkout from an earlier attempt; it never deletes
/// anything. A stale directory at the planned path is reported, not removed.
pub(crate) fn provision_isolated(
    plan: &IsolatedPlan,
    ctx: &crate::vcs::GitContext,
) -> Result<ProvisionedPlacement, String> {
    let listed = crate::vcs::list_worktrees_with(
        &crate::vcs::VcsListWorktreesInput {
            cwd: plan.repo.to_string_lossy().into_owned(),
        },
        ctx,
    )?;
    if !listed.is_repo {
        return Err(format!(
            "Repository {} is not a Git repository; it cannot host an isolated worktree.",
            plan.repo.display()
        ));
    }
    let planned = crate::vcs::real_location(&plan.checkout);
    if let Some(entry) = listed
        .worktrees
        .iter()
        .find(|entry| crate::vcs::real_location(Path::new(&entry.path)) == planned)
    {
        let ref_matches = entry.ref_name.as_deref().is_some_and(|name| {
            name == plan.branch || name == format!("refs/heads/{}", plan.branch)
        });
        if !ref_matches {
            return Err(format!(
                "Checkout {} is registered to a different branch. The recorded reservation cannot be reused.",
                entry.path
            ));
        }
        return Ok(ProvisionedPlacement {
            worktree_path: entry.path.clone(),
            branch: plan.branch.clone(),
            head: worktree_head(Path::new(&entry.path), ctx)?,
            base_branch: None,
        });
    }
    if let Some(entry) = listed.worktrees.iter().find(|entry| {
        entry.ref_name.as_deref() == Some(format!("refs/heads/{}", plan.branch).as_str())
            || entry.ref_name.as_deref() == Some(plan.branch.as_str())
    }) {
        return Err(format!(
            "Branch {} is already checked out at {}. The recorded reservation cannot be reused.",
            plan.branch, entry.path
        ));
    }
    if plan.checkout.exists() {
        return Err(format!(
            "Checkout path {} exists but is not a registered worktree. Remove it manually or choose another placement; Shina will not delete it.",
            plan.checkout.display()
        ));
    }
    if crate::vcs::branch_exists(&plan.repo, &plan.branch, ctx) {
        // An earlier attempt created the branch and stopped before the
        // worktree was added. Reuse it rather than renaming.
        crate::vcs::run_git_checked(
            "Placement.adoptBranch",
            &plan.repo,
            &[
                "worktree",
                "add",
                "--",
                &plan.checkout.to_string_lossy(),
                &plan.branch,
            ],
            crate::vcs::DEFAULT_TIMEOUT,
            "git worktree add failed",
            ctx,
        )?;
        return Ok(ProvisionedPlacement {
            worktree_path: plan.checkout.to_string_lossy().into_owned(),
            branch: plan.branch.clone(),
            head: worktree_head(&plan.checkout, ctx)?,
            base_branch: None,
        });
    }
    let (start_ref, base_branch) = match &plan.base {
        PlannedBase::UpstreamBranch(branch) => (branch.clone(), Some(branch.clone())),
        PlannedBase::Commit(commit) => (commit.clone(), plan.base_branch.clone()),
        PlannedBase::Branch(branch) => (branch.clone(), Some(branch.clone())),
        PlannedBase::DefaultBranch => {
            let branch = default_base_branch(&plan.repo, ctx)?
                .ok_or("Could not resolve a default base branch. Pass placement.baseBranch.")?;
            (branch.clone(), Some(branch))
        }
    };
    let created = crate::vcs::create_worktree_with(
        &crate::vcs::VcsCreateWorktreeInput {
            cwd: plan.repo.to_string_lossy().into_owned(),
            ref_name: start_ref,
            new_ref_name: Some(plan.branch.clone()),
            base_ref_name: base_branch.clone(),
            path: Some(plan.checkout.to_string_lossy().into_owned()),
            start_from_origin: Some(plan.start_from_origin),
        },
        &plan.worktrees_dir,
        ctx,
    )?;
    let worktree_path = created.worktree.path;
    let head = worktree_head(Path::new(&worktree_path), ctx)?;
    Ok(ProvisionedPlacement {
        worktree_path,
        branch: plan.branch.clone(),
        head,
        base_branch,
    })
}
