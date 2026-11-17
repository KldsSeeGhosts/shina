//! The logical repository catalog (issue #28). A repository owns canonical
//! remote identities, a default base and optional setup metadata; checkout
//! bindings map it to executor-local clones. Local paths, credentials and
//! platform setup stay with the owning executor and never enter the catalog.
use super::*;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

const MAX_NAME: usize = 128;
const MAX_REMOTE: usize = 512;
const MAX_REMOTES: usize = 16;
const MAX_URL: usize = 2048;
const MAX_BRANCH: usize = 255;
const MAX_SETUP: usize = 4096;
const MAX_REASON: usize = 2000;
const MAX_BINDINGS: usize = 1024;
const MAX_PATH: usize = 32768;
const CLONE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(900);

/// One durable repository. `id` survives checkout moves, machine changes and
/// peer relinks; `remotes` are the canonical "host/path" identities from
/// remote_tasks::canonical_remote, so SSH and HTTPS spellings resolve
/// together while forks and different hosts stay distinct.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LogicalRepository {
    /// Stable logical id: `repo-` plus a uuid assigned at registration.
    pub id: String,
    pub name: String,
    pub remotes: Vec<String>,
    /// The clone source executors use when they have no checkout yet. Its
    /// canonical identity must be one of `remotes`, so a fork URL cannot
    /// smuggle a different repository into the record.
    pub clone_url: String,
    /// The branch isolated placements start from by default.
    #[serde(default)]
    pub base_branch: Option<String>,
    /// Executor-agnostic setup note. Platform-specific setup and secrets
    /// stay on the executor and are never recorded here.
    #[serde(default)]
    pub setup: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
}

/// Durable binding between a logical repository and one executor-local
/// checkout. `machine` is `local` or a peer's stable server id; `project` and
/// `path` are meaningful only on that machine.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CheckoutBinding {
    pub repository: String,
    pub machine: String,
    #[serde(default)]
    pub project: Option<u64>,
    #[serde(default)]
    pub path: String,
    pub state: BindingState,
    /// The distinct reason a binding is provisioning or failed: no clone yet,
    /// unavailable credentials, a conflicting directory or a clone error.
    #[serde(default)]
    pub reason: Option<String>,
    pub updated_at: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BindingState {
    Bound,
    Provisioning,
    Failed,
}

/// The repository a controller asks an executor to bind in
/// `peer.thread.launch`, validated on the executor before any clone starts.
#[derive(Clone, Debug)]
pub struct LaunchRepository {
    pub id: String,
    pub name: String,
    pub remotes: Vec<String>,
    pub clone_url: String,
    pub base_branch: Option<String>,
}

/// One checkout materialization attempt. The target is deterministic per
/// repository, so retries adopt a completed clone and remove only this
/// job's own staging directory.
pub(crate) struct CheckoutJob {
    pub clone_url: String,
    pub remotes: Vec<String>,
    pub target: PathBuf,
}

fn bounded(value: &str, max: usize, what: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.chars().count() > max || trimmed.chars().any(char::is_control)
    {
        Err(format!("{what} must be 1-{max} printable characters."))
    } else {
        Ok(trimmed.to_owned())
    }
}

fn valid_repository_id(id: &str) -> bool {
    id.starts_with("repo-")
        && id.len() <= 72
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Light branch-name validation mirroring `git check-ref-format --branch`.
fn valid_branch(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= MAX_BRANCH
        && !name
            .chars()
            .any(|c| c.is_control() || " ~^:?*[\\".contains(c))
        && !name.contains("..")
        && !name.contains("@{")
        && name != "@"
        && !name.starts_with('-')
        && !name.starts_with('/')
        && !name.ends_with('/')
        && !name.ends_with('.')
        && !name.ends_with(".lock")
}

/// The canonical remote identity of a clone source. GitHub shorthand maps to
/// its github.com identity; everything else must canonicalize directly.
pub(crate) fn canonical_of_source(source: &crate::projects::Source) -> Result<String, String> {
    match source {
        crate::projects::Source::GitHub { owner, repo } => Ok(format!("github.com/{owner}/{repo}")),
        crate::projects::Source::Url(url) => remote_tasks::canonical_remote(url).ok_or_else(|| {
            "That repository URL does not map to a canonical remote identity.".into()
        }),
    }
}

fn canonical_of_input(input: &str) -> Option<String> {
    remote_tasks::canonical_remote(input).or_else(|| {
        crate::projects::parse_source(input)
            .ok()
            .and_then(|source| canonical_of_source(&source).ok())
    })
}

/// Whether a stored remote is already in canonical `host/path` form. The
/// canonicalizer needs a transport to parse, so the check wraps the value in
/// an HTTPS URL: a default port can never appear in canonical form, while
/// any other explicit port survives.
fn is_canonical_remote(remote: &str) -> bool {
    !remote.is_empty()
        && remote.len() <= MAX_REMOTE
        && !remote.chars().any(char::is_control)
        && remote_tasks::canonical_remote(&format!("https://{remote}")).as_deref() == Some(remote)
}

impl LogicalRepository {
    pub fn value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    fn valid(&self) -> bool {
        valid_repository_id(&self.id)
            && !self.name.is_empty()
            && self.name.chars().count() <= MAX_NAME
            && !self.name.chars().any(char::is_control)
            && !self.remotes.is_empty()
            && self.remotes.len() <= MAX_REMOTES
            && self.remotes.iter().all(|r| is_canonical_remote(r))
            && self
                .remotes
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                == self.remotes.len()
            && self.clone_url.len() <= MAX_URL
            && !self.clone_url.chars().any(char::is_control)
            && self.base_branch.as_deref().is_none_or(|b| valid_branch(b))
            && self
                .setup
                .as_deref()
                .is_none_or(|s| s.chars().count() <= MAX_SETUP && !s.chars().any(char::is_control))
    }

    fn from_value(value: &Value) -> Option<Self> {
        serde_json::from_value::<Self>(value.clone())
            .ok()
            .filter(Self::valid)
    }

    /// The value a controller sends in `peer.thread.launch` and records on
    /// the remote task.
    pub(super) fn launch_value(&self) -> Value {
        json!({
            "id":self.id,
            "name":self.name,
            "remotes":self.remotes,
            "cloneUrl":self.clone_url,
            "baseBranch":self.base_branch,
        })
    }
}

impl CheckoutBinding {
    fn valid(&self, repositories: &[LogicalRepository]) -> bool {
        repositories.iter().any(|r| r.id == self.repository)
            && !self.machine.is_empty()
            && self.machine.chars().count() <= 128
            && !self.machine.chars().any(char::is_control)
            && self
                .project
                .is_none_or(|id| id > 0 && id < usize::MAX as u64)
            && self.path.chars().count() <= MAX_PATH
            && !self.path.chars().any(char::is_control)
            && self
                .reason
                .as_deref()
                .is_none_or(|r| r.chars().count() <= MAX_REASON)
    }
}

/// Parse and validate the `repository` launch argument on the executor.
/// The canonical identities are re-verified, never trusted.
pub(crate) fn parse_launch_repository(value: &Value) -> Result<LaunchRepository, String> {
    let object = value
        .as_object()
        .ok_or("repository must be an object with id, remotes and cloneUrl.")?;
    let id = object["id"]
        .as_str()
        .filter(|id| valid_repository_id(id))
        .ok_or("repository.id must be a catalog repository id.")?
        .to_owned();
    let remotes_value = object["remotes"]
        .as_array()
        .ok_or("repository.remotes must list canonical remote identities.")?;
    if remotes_value.is_empty() || remotes_value.len() > MAX_REMOTES {
        return Err("repository.remotes must list 1-16 canonical remotes.".into());
    }
    let mut remotes = Vec::new();
    for remote in remotes_value {
        let remote = remote
            .as_str()
            .filter(|r| is_canonical_remote(r))
            .ok_or("repository.remotes entries must be canonical remote identities.")?;
        if !remotes.contains(&remote.to_owned()) {
            remotes.push(remote.to_owned());
        }
    }
    let clone_url = bounded(
        object["cloneUrl"]
            .as_str()
            .ok_or("repository.cloneUrl is required.")?,
        MAX_URL,
        "repository.cloneUrl",
    )?;
    let source = crate::projects::parse_source(&clone_url)
        .map_err(|_| "repository.cloneUrl is not a source Shina can clone.".to_string())?;
    let canonical = canonical_of_source(&source)
        .map_err(|_| "repository.cloneUrl does not map to a canonical remote.".to_string())?;
    if !remotes.contains(&canonical) {
        return Err("repository.cloneUrl does not belong to this repository's remotes.".into());
    }
    let name = match object.get("name") {
        None | Some(Value::Null) => default_name(&remotes[0]),
        Some(value) => bounded(
            value.as_str().ok_or("repository.name must be text.")?,
            MAX_NAME,
            "repository.name",
        )?,
    };
    let base_branch = match object.get("baseBranch") {
        None | Some(Value::Null) => None,
        Some(value) => {
            let branch = value
                .as_str()
                .filter(|b| valid_branch(b))
                .ok_or("repository.baseBranch must be a valid Git branch name.")?;
            Some(branch.to_owned())
        }
    };
    Ok(LaunchRepository {
        id,
        name,
        remotes,
        clone_url,
        base_branch,
    })
}

fn default_name(remote: &str) -> String {
    bounded(
        remote.rsplit('/').next().unwrap_or("repository"),
        MAX_NAME,
        "repository.name",
    )
    .unwrap_or_else(|_| "repository".into())
}

/// The deterministic checkout target for one repository on this executor.
/// Retries, restarts and reprovisioning all resolve to this same directory,
/// which keeps provisioning idempotent.
pub(crate) fn checkout_target(data_dir: &Path, id: &str, name: &str) -> PathBuf {
    let slug: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches(['-', '.'])
        .chars()
        .take(48)
        .collect();
    let slug = if slug.is_empty() { "repository" } else { &slug };
    let suffix: String = id
        .strip_prefix("repo-")
        .unwrap_or(id)
        .chars()
        .take(8)
        .collect();
    // The canonical base keeps the recorded checkout identical to the path a
    // provisioned clone is later registered under.
    let base = data_dir
        .canonicalize()
        .unwrap_or_else(|_| data_dir.to_path_buf());
    base.join("checkouts").join(format!("{slug}-{suffix}"))
}

fn remote_at(path: &Path) -> Option<String> {
    let output = crate::process::output(
        std::process::Command::new("git").arg("-C").arg(path).args([
            "config",
            "--get",
            "remote.origin.url",
        ]),
        std::time::Duration::from_secs(2),
        8192,
    )
    .ok()?;
    output
        .status
        .success()
        .then(|| remote_tasks::canonical_remote(String::from_utf8_lossy(&output.stdout).trim()))
        .flatten()
}

/// Materialize the recorded checkout: adopt an existing matching clone, or
/// clone into a staging sibling and publish it to the target. Only the
/// executor's existing Git credentials are used; prompting stays disabled,
/// so unavailable credentials surface as a distinct failure reason.
pub(crate) fn provision_checkout(job: &CheckoutJob) -> Result<PathBuf, String> {
    let target = &job.target;
    if target.exists() {
        match remote_at(target) {
            Some(remote) if job.remotes.contains(&remote) => return Ok(target.clone()),
            Some(remote) => {
                return Err(format!(
                    "{} is already a checkout of {remote}, not this repository.",
                    target.display()
                ));
            }
            None => {
                let empty = std::fs::read_dir(target)
                    .map_err(|e| format!("Could not read {}: {e}", target.display()))?
                    .next()
                    .is_none();
                if !empty {
                    return Err(format!(
                        "{} exists and is not a checkout of this repository.",
                        target.display()
                    ));
                }
                std::fs::remove_dir(target).map_err(|e| {
                    format!(
                        "Could not clear the empty checkout folder {}: {e}",
                        target.display()
                    )
                })?;
            }
        }
    }
    let parent = target.parent().ok_or("Invalid checkout target.")?;
    std::fs::create_dir_all(parent)
        .map_err(|e| format!("Could not create {}: {e}", parent.display()))?;
    let name = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("checkout");
    let staging = parent.join(format!("{name}.clone-tmp"));
    if staging.exists() {
        // A leftover staging directory belongs to an interrupted attempt.
        std::fs::remove_dir_all(&staging).map_err(|e| {
            format!(
                "Could not clear the staged clone {}: {e}",
                staging.display()
            )
        })?;
    }
    let source = crate::projects::parse_source(&job.clone_url)
        .map_err(|_| "The repository's clone URL is not a source Shina can clone.".to_string())?;
    let mut command = crate::projects::clone_command(&source, &staging);
    let output = crate::process::output(&mut command, CLONE_TIMEOUT, 64 * 1024).map_err(|e| {
        if e.kind() == std::io::ErrorKind::TimedOut {
            "Clone timed out.".to_string()
        } else {
            format!("Clone could not start: {e}")
        }
    })?;
    if !output.status.success() {
        let _ = std::fs::remove_dir_all(&staging);
        let stderr = crate::projects::redact(&String::from_utf8_lossy(&output.stderr));
        let tail = stderr
            .lines()
            .rev()
            .take(4)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join(" ");
        return Err(format!("Clone failed ({}): {}", output.status, tail.trim()));
    }
    if let Err(e) = std::fs::rename(&staging, target) {
        let _ = std::fs::remove_dir_all(&staging);
        if remote_at(target).is_some_and(|r| job.remotes.contains(&r)) {
            return Ok(target.clone());
        }
        return Err(format!(
            "Could not publish the clone to {}: {e}",
            target.display()
        ));
    }
    match remote_at(target) {
        Some(remote) if job.remotes.contains(&remote) => Ok(target.clone()),
        _ => Err(format!(
            "{} did not come up as a checkout of this repository.",
            target.display()
        )),
    }
}

/// The catalog fields stored in the index: repositories, bindings and
/// captain repository scopes. Shared by restore validation and sidecar sync.
pub(crate) fn catalog_from_index(
    index: &Value,
) -> Result<
    (
        Vec<LogicalRepository>,
        Vec<CheckoutBinding>,
        BTreeMap<usize, Vec<String>>,
    ),
    String,
> {
    let repositories: Vec<LogicalRepository> = match index.get("repositories") {
        None => Vec::new(),
        Some(value) => value
            .as_array()
            .ok_or("Invalid repository catalog.")?
            .iter()
            .map(LogicalRepository::from_value)
            .collect::<Option<Vec<_>>>()
            .ok_or("Invalid repository record.")?,
    };
    for (position, repository) in repositories.iter().enumerate() {
        if repositories[..position]
            .iter()
            .any(|r| r.id == repository.id)
        {
            return Err("Duplicate repository ids.".into());
        }
        if repositories[..position].iter().any(|r| {
            r.remotes
                .iter()
                .any(|remote| repository.remotes.contains(remote))
        }) {
            return Err("A canonical remote may name only one repository.".into());
        }
    }
    let bindings: Vec<CheckoutBinding> = match index.get("repositoryBindings") {
        None => Vec::new(),
        Some(value) => {
            serde_json::from_value(value.clone()).map_err(|_| "Invalid repository bindings.")?
        }
    };
    if bindings.len() > MAX_BINDINGS || bindings.iter().any(|b| !b.valid(&repositories)) {
        return Err("Invalid repository bindings.".into());
    }
    let scopes: BTreeMap<usize, Vec<String>> = match index.get("captainRepositoryScopes") {
        None => BTreeMap::new(),
        Some(value) => serde_json::from_value(value.clone())
            .map_err(|_| "Invalid captain repository scopes.")?,
    };
    for ids in scopes.values() {
        if ids
            .iter()
            .any(|id| !repositories.iter().any(|r| &r.id == id))
        {
            return Err("Captain repository scope names an unknown repository.".into());
        }
    }
    Ok((repositories, bindings, scopes))
}

/// Mirror the committed index's catalog into the SQLite sidecar inside the
/// same write transaction.
pub(crate) fn sync_index(connection: &Connection, index: &Value) -> Result<(), String> {
    if index["_partial"] == true || index.get("repositories").is_none() {
        return Ok(());
    }
    let (repositories, bindings, scopes) = catalog_from_index(index)?;
    connection
        .execute_batch(
            "DELETE FROM catalog_captain_scopes;DELETE FROM catalog_checkouts;
             DELETE FROM catalog_repository_remotes;DELETE FROM catalog_repositories;",
        )
        .map_err(|e| e.to_string())?;
    for repository in &repositories {
        connection
            .execute(
                "INSERT INTO catalog_repositories
                 (repository_id,name,clone_url,base_branch,setup,created_at,updated_at)
                 VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![
                    repository.id,
                    repository.name,
                    repository.clone_url,
                    repository.base_branch,
                    repository.setup,
                    repository.created_at as i64,
                    repository.updated_at as i64
                ],
            )
            .map_err(|e| e.to_string())?;
        for remote in &repository.remotes {
            connection
                .execute(
                    "INSERT INTO catalog_repository_remotes (canonical_remote,repository_id)
                     VALUES(?1,?2)",
                    params![remote, repository.id],
                )
                .map_err(|e| e.to_string())?;
        }
    }
    for binding in &bindings {
        connection
            .execute(
                "INSERT INTO catalog_checkouts
                 (repository_id,machine,project_id,path,state,reason,updated_at)
                 VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![
                    binding.repository,
                    binding.machine,
                    binding.project.map(|id| id as i64),
                    binding.path,
                    match binding.state {
                        BindingState::Bound => "bound",
                        BindingState::Provisioning => "provisioning",
                        BindingState::Failed => "failed",
                    },
                    binding.reason,
                    binding.updated_at as i64
                ],
            )
            .map_err(|e| e.to_string())?;
    }
    for (captain, repositories) in &scopes {
        for (position, id) in repositories.iter().enumerate() {
            connection
                .execute(
                    "INSERT INTO catalog_captain_scopes (captain,repository_id,position)
                     VALUES(?1,?2,?3)",
                    params![*captain as i64, id, position as i64],
                )
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Rebuild the catalog sidecar from the committed index at open. The index is
/// authoritative; rows that no longer appear there are dropped.
pub(crate) fn validate_open(connection: &Connection) -> Result<(), String> {
    let tx = connection
        .unchecked_transaction()
        .map_err(|e| e.to_string())?;
    if let Some((index, _)) = crate::identity::read_index(&tx)? {
        sync_index(&tx, &index)?;
    } else {
        tx.execute_batch(
            "DELETE FROM catalog_captain_scopes;DELETE FROM catalog_checkouts;
             DELETE FROM catalog_repository_remotes;DELETE FROM catalog_repositories;",
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())
}

impl State {
    /// Resolve a catalog id or any spelling of a repository URL to a record.
    pub(super) fn repository_index_by_selector(&self, selector: &Value) -> Result<usize, String> {
        let selector = selector
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or("repository must name a catalog id or repository URL.")?;
        if let Some(index) = self.repositories.iter().position(|r| r.id == selector) {
            return Ok(index);
        }
        let canonical = canonical_of_input(selector)
            .ok_or("repository must be a catalog id or a repository URL.")?;
        self.repositories
            .iter()
            .position(|r| r.remotes.contains(&canonical))
            .ok_or_else(|| format!("No catalog repository matches {selector}."))
    }

    /// Register or update a repository, and optionally assign it to a
    /// captain's repository scope. A URL whose canonical remote already
    /// belongs to a repository merges into that record; two different
    /// repositories are never conflated.
    pub(super) fn put_repository(&mut self, args: &Value) -> Result<Value, String> {
        if let Some(result) = self.remote("repository.put", args.clone()) {
            return result.map(|_| Value::Null);
        }
        let url = bounded(
            args["url"].as_str().ok_or("repository.url is required.")?,
            MAX_URL,
            "repository.url",
        )?;
        let source = crate::projects::parse_source(&url)?;
        let primary = canonical_of_source(&source)?;
        let mut remotes = vec![primary.clone()];
        if let Some(extras) = args.get("remotes") {
            let extras = extras
                .as_array()
                .ok_or("repository.remotes must be an array of remote URLs.")?;
            for extra in extras {
                let extra = extra
                    .as_str()
                    .ok_or("repository.remotes entries must be remote URLs.")?;
                let canonical = canonical_of_input(extra)
                    .ok_or("repository.remotes entries must be repository URLs.")?;
                if !remotes.contains(&canonical) {
                    remotes.push(canonical);
                }
            }
        }
        if remotes.len() > MAX_REMOTES {
            return Err(format!(
                "A repository may carry at most {MAX_REMOTES} remotes."
            ));
        }
        let name = match args.get("name") {
            None | Some(Value::Null) => default_name(&primary),
            Some(value) => bounded(
                value.as_str().ok_or("repository.name must be text.")?,
                MAX_NAME,
                "repository.name",
            )?,
        };
        let base_branch = match args.get("baseBranch") {
            None | Some(Value::Null) => None,
            Some(value) => Some(
                value
                    .as_str()
                    .filter(|b| valid_branch(b))
                    .ok_or("repository.baseBranch must be a valid Git branch name.")?
                    .to_owned(),
            ),
        };
        let setup = match args.get("setup") {
            None | Some(Value::Null) => None,
            Some(value) => Some(bounded(
                value.as_str().ok_or("repository.setup must be text.")?,
                MAX_SETUP,
                "repository.setup",
            )?),
        };
        let captain = match args.get("captain") {
            None | Some(Value::Null) => None,
            Some(value) => Some(
                value
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .ok_or("repository.captain must be a captain id.")?,
            ),
        };
        if let Some(captain) = captain {
            self.require_active_captain(captain)?;
        }
        let claimers: Vec<usize> = self
            .repositories
            .iter()
            .enumerate()
            .filter(|(_, r)| r.remotes.iter().any(|remote| remotes.contains(remote)))
            .map(|(index, _)| index)
            .collect();
        if claimers.len() > 1 {
            return Err("Those remotes already belong to different catalog repositories.".into());
        }
        let now = lifecycle::now();
        let index = match claimers.first() {
            Some(&index) => {
                let repository = &mut self.repositories[index];
                for remote in remotes {
                    if !repository.remotes.contains(&remote) {
                        repository.remotes.push(remote);
                    }
                }
                if args.get("name").is_some_and(|v| v.is_string()) {
                    repository.name = name;
                }
                if args.get("baseBranch").is_some_and(|v| v.is_string()) {
                    repository.base_branch = base_branch;
                }
                if args.get("setup").is_some_and(|v| v.is_string()) {
                    repository.setup = setup;
                }
                repository.updated_at = now;
                index
            }
            None => {
                let mut id = format!("repo-{}", uuid::Uuid::new_v4());
                while self.repositories.iter().any(|r| r.id == id) {
                    id = format!("repo-{}", uuid::Uuid::new_v4());
                }
                self.repositories.push(LogicalRepository {
                    id,
                    name,
                    remotes,
                    clone_url: url.clone(),
                    base_branch,
                    setup,
                    created_at: now,
                    updated_at: now,
                });
                self.repositories.len() - 1
            }
        };
        if let Some(captain) = captain {
            let id = self.repositories[index].id.clone();
            let scope = self.captain_repository_scopes.entry(captain).or_default();
            if !scope.contains(&id) {
                scope.push(id);
            }
        }
        self.refresh_local_bindings();
        self.checkpoint();
        Ok(self.repositories[index].value())
    }

    pub(super) fn remove_repository(&mut self, selector: &Value) -> Result<(), String> {
        if let Some(result) =
            self.remote("repository.remove", json!({"repository":selector.clone()}))
        {
            return result.map(|_| ());
        }
        let index = self.repository_index_by_selector(selector)?;
        let id = self.repositories.remove(index).id;
        self.repository_bindings
            .retain(|binding| binding.repository != id);
        for scope in self.captain_repository_scopes.values_mut() {
            scope.retain(|repository| *repository != id);
        }
        self.checkpoint();
        Ok(())
    }

    /// Read the catalog with every repository's bindings and captain
    /// assignments. `repository.list` (read grant) and the
    /// `list_repositories` tool share this view.
    pub(super) fn repository_list_value(&self) -> Value {
        json!({
            "repositories":self.repositories.iter().map(|repository|{
                let mut value = repository.value();
                value["bindings"] = json!(self.repository_bindings.iter()
                    .filter(|b|b.repository==repository.id).collect::<Vec<_>>());
                value["captains"] = json!(self.captain_repository_scopes.iter()
                    .filter(|(_,ids)|ids.contains(&repository.id))
                    .map(|(captain,_)|*captain).collect::<Vec<_>>());
                value
            }).collect::<Vec<_>>(),
            "feature":crate::protocol::LOGICAL_REPOSITORIES
        })
    }

    pub fn captain_repository_scope(&self, captain: usize) -> Vec<String> {
        self.captain_repository_scopes
            .get(&captain)
            .cloned()
            .unwrap_or_default()
    }

    /// Replace a captain's repository scope. Selectors may be catalog ids or
    /// repository URLs; an unregistered URL registers the repository first.
    pub(super) fn set_captain_repository_scope(
        &mut self,
        captain: usize,
        selectors: &Value,
    ) -> Result<(), String> {
        if let Some(result) = self.remote(
            "captain.scope",
            json!({
                "captain":captain,
                "projects":self.captain_scope(captain),
                "remoteProjects":self.remote_captain_scope(captain),
                "repositories":selectors.clone()
            }),
        ) {
            return result.map(|_| ());
        }
        self.require_active_captain(captain)?;
        if self
            .captain_home(captain)
            .and_then(|id| self.threads.iter().find(|t| t.id == id))
            .is_some_and(|t| t.status.busy() || t.configuring || t.queued())
        {
            return Err("Stop the captain's current turn before changing its scope.".into());
        }
        let selectors = selectors
            .as_array()
            .ok_or("repositories must be an array of catalog ids or repository URLs.")?;
        let mut ids = Vec::new();
        for selector in selectors {
            let index = match self.repository_index_by_selector(selector) {
                Ok(index) => index,
                Err(_) => {
                    // A URL that names no repository registers it, so a
                    // captain can be assigned a remote-only repository.
                    let url = selector
                        .as_str()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .ok_or("repositories must name catalog ids or repository URLs.")?;
                    self.put_repository(&json!({"url":url}))?;
                    self.repository_index_by_selector(selector)?
                }
            };
            let id = self.repositories[index].id.clone();
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        self.captain_repository_scopes.insert(captain, ids);
        if let Some(thread) = self
            .captain_home(captain)
            .and_then(|id| self.threads.iter_mut().find(|t| t.id == id))
        {
            thread.scope_context_pending = true;
        }
        self.checkpoint();
        Ok(())
    }

    /// Whether the caller's local project is a bound checkout of the
    /// repository: a recorded local binding, or a warm Git identity match.
    pub(super) fn project_binds_repository(
        &self,
        project: ProjectId,
        repository: &LogicalRepository,
    ) -> bool {
        self.repository_bindings.iter().any(|binding| {
            binding.repository == repository.id
                && binding.machine == "local"
                && binding.project == Some(project as u64)
                && binding.state != BindingState::Failed
        }) || self.project(Some(project)).is_some_and(|project| {
            remote_tasks::repository(&project.path)
                .is_some_and(|remote| repository.remotes.contains(&remote))
        })
    }

    /// Record that a bound or provisioning checkout serves a repository on a
    /// machine. The map is refreshed, never trusted as live capability.
    pub(super) fn record_binding(
        &mut self,
        repository: &str,
        machine: &str,
        project: Option<u64>,
        path: &str,
        state: BindingState,
        reason: Option<String>,
    ) {
        let now = lifecycle::now();
        if let Some(binding) = self
            .repository_bindings
            .iter_mut()
            .find(|b| b.repository == repository && b.machine == machine && b.path == path)
        {
            binding.project = project.or(binding.project);
            binding.state = state;
            binding.reason = reason;
            binding.updated_at = now;
        } else {
            self.repository_bindings.push(CheckoutBinding {
                repository: repository.into(),
                machine: machine.into(),
                project,
                path: path.into(),
                state,
                reason,
                updated_at: now,
            });
        }
        if self.repository_bindings.len() > MAX_BINDINGS {
            self.repository_bindings
                .sort_by_key(|b| std::cmp::Reverse(b.updated_at));
            self.repository_bindings.truncate(MAX_BINDINGS);
        }
    }

    /// Attach every warm local checkout identity to a matching repository.
    /// Cold lookups warm in the background and bind on the next pass.
    pub(super) fn refresh_local_bindings(&mut self) {
        if self.client.is_some() {
            // A client never probes the owning server's paths.
            return;
        }
        let checkouts: Vec<(u64, String, String)> = self
            .projects
            .entries
            .iter()
            .filter(|p| p.machine == "local")
            .filter_map(|p| {
                remote_tasks::repository(&p.path)
                    .map(|remote| (p.id as u64, p.path.to_string_lossy().into_owned(), remote))
            })
            .collect();
        for index in 0..self.repositories.len() {
            let (id, remotes) = {
                let repository = &self.repositories[index];
                (repository.id.clone(), repository.remotes.clone())
            };
            for (project, path, remote) in &checkouts {
                if remotes.contains(remote)
                    && !self.repository_bindings.iter().any(|binding| {
                        binding.repository == id
                            && binding.machine == "local"
                            && binding.path == *path
                            && binding.state == BindingState::Bound
                    })
                {
                    self.record_binding(
                        &id,
                        "local",
                        Some(*project),
                        path,
                        BindingState::Bound,
                        None,
                    );
                }
            }
        }
    }

    /// Executor side: keep a local catalog row for the repository a peer
    /// launch named, preferring an existing local record for its remotes.
    /// Returns the local repository id used for binding bookkeeping.
    pub(super) fn learn_repository(&mut self, launch: &LaunchRepository) -> String {
        if let Some(index) = self.repositories.iter().position(|r| r.id == launch.id) {
            let repository = &mut self.repositories[index];
            for remote in &launch.remotes {
                if !repository.remotes.contains(remote) {
                    repository.remotes.push(remote.clone());
                }
            }
            repository.updated_at = lifecycle::now();
            return repository.id.clone();
        }
        if let Some(index) = self.repositories.iter().position(|r| {
            r.remotes
                .iter()
                .any(|remote| launch.remotes.contains(remote))
        }) {
            return self.repositories[index].id.clone();
        }
        let now = lifecycle::now();
        self.repositories.push(LogicalRepository {
            id: launch.id.clone(),
            name: launch.name.clone(),
            remotes: launch.remotes.clone(),
            clone_url: launch.clone_url.clone(),
            base_branch: launch.base_branch.clone(),
            setup: None,
            created_at: now,
            updated_at: now,
        });
        launch.id.clone()
    }

    /// Register the executor-local project wrapping a provisioned checkout
    /// without opening it or changing any selection.
    fn register_checkout_project(&mut self, path: &Path) -> usize {
        let path = crate::projects::clean_path(
            &path.canonicalize().unwrap_or_else(|_| path.to_path_buf()),
        );
        if let Some(project) = self
            .projects
            .entries
            .iter_mut()
            .find(|p| p.machine == "local" && crate::projects::same_path(&p.path, &path))
        {
            project.hidden = false;
            project.updated = crate::projects::now();
            return project.id;
        }
        let id = self.projects.next_id.max(1);
        self.projects.next_id = id + 1;
        self.projects.entries.push(Project::new(id, path.clone()));
        remote_tasks::repository(&path);
        id
    }

    /// Restore the catalog from a committed index. Called after captains are
    /// validated, before any runtime uses the records.
    pub(super) fn restore_repository_records(&mut self, index: &Value) -> Result<(), String> {
        let (repositories, bindings, scopes) = catalog_from_index(index)?;
        for captain in scopes.keys() {
            self.require_captain(*captain)?;
        }
        self.repositories = repositories;
        self.repository_bindings = bindings;
        self.captain_repository_scopes = scopes;
        // Preserve existing machine-local projects as checkout bindings
        // wherever their Git identity is already resolved.
        self.refresh_local_bindings();
        Ok(())
    }

    fn checkout_provision_job(&self, index: usize) -> Option<CheckoutJob> {
        let binding = self.threads[index].placement.as_ref()?.binding.as_ref()?;
        if binding.state != placement::BindingPhase::Pending {
            return None;
        }
        let target = if binding.checkout.is_empty() {
            checkout_target(&self.data_dir, &binding.repository, "repository")
        } else {
            PathBuf::from(&binding.checkout)
        };
        Some(CheckoutJob {
            clone_url: binding.clone_url.clone(),
            remotes: binding.remotes.clone(),
            target,
        })
    }

    /// Drive pending checkout bindings: clone or adopt the recorded target,
    /// register the project and run the deferred placement plan. Serialized
    /// one job at a time like worktree provisioning; only the owning server
    /// clones, never a projecting client.
    pub(super) fn poll_checkout_provisioning(&mut self) -> bool {
        if self.client.is_some() {
            return false;
        }
        let mut changed = false;
        let mut remaining = Vec::with_capacity(self.checkout_rxs.len());
        for (thread_id, rx) in std::mem::take(&mut self.checkout_rxs) {
            match rx.try_recv() {
                Ok(result) => {
                    changed = true;
                    self.apply_checkout_outcome(thread_id, result);
                }
                Err(mpsc::TryRecvError::Empty) => remaining.push((thread_id, rx)),
                Err(mpsc::TryRecvError::Disconnected) => {
                    changed = true;
                    self.apply_checkout_outcome(
                        thread_id,
                        Err("Checkout provisioning stopped unexpectedly.".into()),
                    );
                }
            }
        }
        self.checkout_rxs = remaining;
        for index in 0..self.threads.len() {
            let thread_id = self.threads[index].id;
            if self.checkout_rxs.iter().any(|(id, _)| *id == thread_id) {
                continue;
            }
            let Some(job) = self.checkout_provision_job(index) else {
                continue;
            };
            if !self.placement_wanted(&self.threads[index]) {
                continue;
            }
            if !self.checkout_rxs.is_empty() {
                break;
            }
            changed = true;
            let wake = self.wake.clone();
            let (tx, rx) = mpsc::channel();
            self.checkout_rxs.push((thread_id, rx));
            std::thread::spawn(move || {
                let _ = tx.send(provision_checkout(&job));
                wake();
            });
        }
        if changed {
            self.persistence_scope = None;
        }
        changed
    }

    fn apply_checkout_outcome(&mut self, thread_id: usize, result: Result<PathBuf, String>) {
        let Some(index) = self.threads.iter().position(|t| t.id == thread_id) else {
            return;
        };
        let Some(binding) = self.threads[index]
            .placement
            .as_ref()
            .and_then(|p| p.binding.clone())
        else {
            return;
        };
        if binding.state != placement::BindingPhase::Pending {
            return;
        }
        match result {
            Ok(path) => {
                if !self.placement_wanted(&self.threads[index]) {
                    return;
                }
                let project_path = crate::projects::clean_path(
                    &path.canonicalize().unwrap_or_else(|_| path.clone()),
                );
                let project = self.register_checkout_project(&project_path);
                self.threads[index].project = Some(project);
                let local = self.learn_repository(&LaunchRepository {
                    id: binding.repository.clone(),
                    name: binding
                        .remotes
                        .first()
                        .map(|remote| default_name(remote))
                        .unwrap_or_else(|| "repository".into()),
                    remotes: binding.remotes.clone(),
                    clone_url: binding.clone_url.clone(),
                    base_branch: None,
                });
                self.record_binding(
                    &local,
                    "local",
                    Some(project as u64),
                    &project_path.to_string_lossy(),
                    BindingState::Bound,
                    None,
                );
                let seed = self.threads[index]
                    .peer_origin
                    .as_ref()
                    .map(|origin| format!("peer:{}", origin.task_id))
                    .unwrap_or_else(|| format!("checkout:{thread_id}"));
                let executor = std::fs::read_to_string(self.data_dir.join("server-id"))
                    .map(|id| id.trim().to_owned())
                    .unwrap_or_else(|_| "local".into());
                let spec = binding.spec.clone();
                #[cfg(test)]
                let git_ctx = self.test_git_context.clone().unwrap_or_default();
                #[cfg(not(test))]
                let git_ctx = crate::vcs::GitContext::default();
                let resolved = self.plan_thread_placement(
                    &self.threads[index],
                    spec.as_ref(),
                    &seed,
                    &executor,
                    &git_ctx,
                );
                match resolved {
                    Ok(mut resolved) => {
                        resolved.binding = Some(placement::PlacementBinding {
                            repository: binding.repository.clone(),
                            remotes: binding.remotes.clone(),
                            clone_url: String::new(),
                            checkout: project_path.to_string_lossy().into_owned(),
                            state: placement::BindingPhase::Ready,
                            spec: None,
                            error: None,
                        });
                        if resolved.read_only() {
                            self.threads[index].native.access = AccessMode::ReadOnly;
                        }
                        if resolved.uses_worktree() {
                            let thread = &mut self.threads[index];
                            thread.worktree_path = resolved.checkout.clone();
                            thread.branch = resolved.branch.clone();
                            thread.env_mode = Some(ThreadEnvMode::Worktree);
                        }
                        let pending = resolved.phase == placement::PlacementPhase::Pending;
                        self.threads[index].placement = Some(resolved);
                        if !pending {
                            self.finish_placement_ready(index);
                        }
                    }
                    Err(error) => {
                        self.fail_checkout_binding(index, error);
                    }
                }
            }
            Err(error) => self.fail_checkout_binding(index, error),
        }
    }

    /// Provisioning or deferred planning failed before a provider started.
    /// The thread keeps its binding and the failure reason; a delivery retry
    /// reprovisions the same recorded target.
    fn fail_checkout_binding(&mut self, index: usize, error: String) {
        if let Some(placement) = self.threads[index].placement.as_mut() {
            placement.phase = placement::PlacementPhase::Failed;
            placement.error = Some(error.clone());
            if let Some(binding) = placement.binding.as_mut() {
                binding.state = placement::BindingPhase::Failed;
                binding.error = Some(error.clone());
            }
        }
        self.threads[index].fail_session(format!("Checkout provisioning failed: {error}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Command;
    use std::time::{Duration, Instant};

    fn git_checkout(path: &Path, remote: &str) {
        std::fs::create_dir_all(path).unwrap();
        for args in [
            vec!["init", "--quiet"],
            vec!["remote", "add", "origin", remote],
        ] {
            assert!(
                std::process::Command::new("git")
                    .arg("-C")
                    .arg(path)
                    .args(&args)
                    .status()
                    .unwrap()
                    .success()
            );
        }
        assert!(
            std::process::Command::new("git")
                .arg("-C")
                .arg(path)
                .args(["-c", "user.email=t@t", "-c", "user.name=t"])
                .args(["commit", "--quiet", "--allow-empty", "-m", "init"])
                .status()
                .unwrap()
                .success()
        );
    }

    fn state() -> State {
        let directory =
            std::env::temp_dir().join(format!("shina-catalog-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join("server-id"),
            uuid::Uuid::new_v4().to_string(),
        )
        .unwrap();
        let mut state = State::headless(directory);
        state.demo = true;
        state.catalog.models = vec![
            AgentModel::from_value(
                &json!({"provider":"cpa","id":"fixture","name":"Fixture","reasoning":true}),
            )
            .unwrap(),
        ];
        state
    }

    /// A captain home delegating to one linked peer that advertises the
    /// catalog contract. `projects` sets the executor's advertised checkouts.
    fn home(projects: Value) -> State {
        let mut state = state();
        state.captains = Captain::presets();
        state.threads[0].captain = Some(0);
        state.threads[0].captain_home = true;
        state.peers.push(crate::peer::Peer {
            server_id: uuid::Uuid::new_v4().to_string(),
            host: "127.0.0.1:1".into(),
            label: "Target".into(),
            linked_at: 0,
            capabilities: json!({
                "contracts":[crate::protocol::PLACEMENT,crate::protocol::LOGICAL_REPOSITORIES],
                "projects":projects,
                "harnesses":[{"id":"pi"}],
                "models":[{"provider":"cpa","id":"fixture","reasoning_levels":["off"]}]}),
            last_reached_at: Some(0),
            last_error: None,
        });
        state
    }

    fn delegate(state: &mut State, repository: &str) -> Result<Value, String> {
        state.mcp_call(
            1,
            "delegate_task",
            &json!({"prompt":"Do the work","target":{
                "machine":"Target","repository":repository,"model":"fixture",
                "options":{"reasoning":"off"}}}),
        )
    }

    #[test]
    fn ssh_and_https_spellings_merge_and_forks_stay_distinct() {
        let mut state = state();
        let first = state
            .put_repository(&json!({"url":"git@example.com:team/repo.git"}))
            .unwrap();
        let second = state
            .put_repository(&json!({"url":"https://example.com/team/repo.git"}))
            .unwrap();
        assert_eq!(first["id"], second["id"]);
        assert_eq!(state.repositories.len(), 1);
        let fork = state
            .put_repository(&json!({"url":"https://example.com/fork/repo.git"}))
            .unwrap();
        assert_ne!(fork["id"], first["id"]);
        assert_eq!(state.repositories.len(), 2);
        let other_host = state
            .put_repository(&json!({"url":"git@another.example.com:team/repo.git"}))
            .unwrap();
        assert_ne!(other_host["id"], first["id"]);
        assert_eq!(state.repositories.len(), 3);
        std::fs::remove_dir_all(state.data_dir).unwrap();
    }

    #[test]
    fn catalog_records_survive_snapshot_and_restore() {
        let mut state = state();
        state.captains = Captain::presets();
        let repository = state
            .put_repository(
                &json!({"url":"git@example.com:team/repo.git","name":"Repo","captain":0}),
            )
            .unwrap();
        let snapshot = state.snapshot();
        let mut restored = State::headless(state.data_dir.clone());
        restored.restore_global(&snapshot).unwrap();
        assert_eq!(restored.repositories, state.repositories);
        assert_eq!(
            restored.captain_repository_scope(0),
            vec![repository["id"].as_str().unwrap().to_owned()]
        );
        std::fs::remove_dir_all(state.data_dir).unwrap();
    }

    #[test]
    fn captain_assignment_needs_no_local_checkout() {
        let mut state = state();
        state.captains = Captain::presets();
        let repository = state
            .put_repository(&json!({"url":"https://example.com/team/repo.git","captain":0}))
            .unwrap();
        assert!(state.projects.entries.is_empty());
        assert_eq!(
            state.captain_repository_scope(0),
            vec![repository["id"].as_str().unwrap().to_owned()]
        );
        let scope = state.scope_value(Some(0));
        assert_eq!(scope[0]["repositoryId"], repository["id"]);
        assert_eq!(scope[0]["cloneUrl"], "https://example.com/team/repo.git");
        std::fs::remove_dir_all(state.data_dir).unwrap();
    }

    #[test]
    fn repository_delegation_provisions_on_an_executor_without_a_checkout() {
        let mut state = home(json!([]));
        let repository = state
            .put_repository(&json!({"url":"https://example.com/team/repo.git","captain":0}))
            .unwrap();
        let id = repository["id"].as_str().unwrap();
        delegate(&mut state, id).unwrap();
        let remote = state.tasks[0].remote.as_ref().unwrap();
        assert_eq!(remote.launch_command.params["projectId"], Value::Null);
        assert_eq!(remote.launch_command.params["repository"]["id"], json!(id));
        assert_eq!(remote.repository.as_ref().unwrap()["id"], json!(id));
        assert_eq!(
            remote.placement.as_ref().unwrap()["phase"],
            json!("pending")
        );
        assert_eq!(
            remote.placement.as_ref().unwrap()["binding"]["state"],
            json!("pending")
        );
        std::fs::remove_dir_all(state.data_dir).unwrap();
    }

    #[test]
    fn repository_delegation_reuses_the_single_bound_checkout() {
        let mut state = home(json!([
            {"id":9,"path":"/target/project","name":"Project","gitRemote":"example.com/team/repo"},
            {"id":10,"path":"/target/other","name":"Other","gitRemote":"example.com/other/thing"}
        ]));
        let repository = state
            .put_repository(&json!({"url":"https://example.com/team/repo.git","captain":0}))
            .unwrap();
        delegate(&mut state, repository["id"].as_str().unwrap()).unwrap();
        let remote = state.tasks[0].remote.as_ref().unwrap();
        assert_eq!(remote.launch_command.params["projectId"], json!(9));
        let binding = &remote.placement.as_ref().unwrap()["binding"];
        assert_eq!(binding["state"], json!("ready"));
        assert_eq!(binding["checkout"], json!("/target/project"));
        std::fs::remove_dir_all(state.data_dir).unwrap();
    }

    #[test]
    fn ambiguous_bindings_and_missing_contracts_fail_distinctly() {
        let mut state = home(json!([
            {"id":9,"path":"/target/one","name":"One","gitRemote":"example.com/team/repo"},
            {"id":10,"path":"/target/two","name":"Two","gitRemote":"example.com/team/repo"}
        ]));
        let repository = state
            .put_repository(&json!({"url":"https://example.com/team/repo.git","captain":0}))
            .unwrap();
        let id = repository["id"].as_str().unwrap().to_owned();
        let error = delegate(&mut state, &id).unwrap_err();
        assert!(error.contains("Several checkouts"), "{error}");
        assert!(error.contains("projectId"), "{error}");
        assert!(state.tasks.is_empty());

        let mut old = home(json!([]));
        old.peers[0].capabilities["contracts"] = json!([crate::protocol::PLACEMENT]);
        old.put_repository(&json!({"url":"https://example.com/team/repo.git","captain":0}))
            .unwrap();
        let old_id = old.repositories[0].id.clone();
        let error = delegate(&mut old, &old_id).unwrap_err();
        assert!(error.contains("Update required"), "{error}");
        assert!(error.contains(crate::protocol::LOGICAL_REPOSITORIES));
        assert!(old.tasks.is_empty());

        let mut unscoped = home(json!([]));
        let repository = unscoped
            .put_repository(&json!({"url":"https://example.com/team/repo.git"}))
            .unwrap();
        let error = delegate(&mut unscoped, repository["id"].as_str().unwrap()).unwrap_err();
        assert!(error.contains("outside the caller's scope"), "{error}");
        std::fs::remove_dir_all(state.data_dir).unwrap();
        std::fs::remove_dir_all(old.data_dir).unwrap();
        std::fs::remove_dir_all(unscoped.data_dir).unwrap();
    }

    #[test]
    fn executor_launch_provisions_then_binds_the_checkout() {
        let mut state = state();
        let repository = json!({"id":"repo-executor-test","name":"repo",
            "remotes":["example.com/team/repo"],"cloneUrl":"https://example.com/team/repo.git"});
        // A completed clone already sits at the deterministic target, so the
        // job must adopt it rather than clone a second copy.
        git_checkout(
            &checkout_target(&state.data_dir, "repo-executor-test", "repo"),
            "https://example.com/team/repo.git",
        );
        let launch = Command::new(
            "peer.thread.launch",
            json!({"projectId":null,"repository":repository,"harness":"pi","provider":"cpa",
                "model":"fixture","brief":"A bounded task","title":"Worker",
                "origin":{"deviceId":"asserted","serverId":uuid::Uuid::new_v4().to_string(),
                    "serverLabel":"Home","parentThreadId":1,"taskId":"task-1",
                    "captain":"Captain","memories":[]},
                "placement":{"mode":"shared"}}),
        );
        let result = state.execute_peer_command("device-1", &launch).unwrap();
        let thread_id = result["threadId"].as_u64().unwrap() as usize;
        let index = state
            .threads
            .iter()
            .position(|t| t.id == thread_id)
            .unwrap();
        let placement = state.threads[index].placement.as_ref().unwrap();
        assert_eq!(placement.phase, placement::PlacementPhase::Pending);
        assert_eq!(
            placement.binding.as_ref().unwrap().state,
            placement::BindingPhase::Pending
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            state.poll_checkout_provisioning();
            let placement = state.threads[index].placement.as_ref().unwrap();
            if placement.binding.as_ref().unwrap().state != placement::BindingPhase::Pending {
                break;
            }
            assert!(Instant::now() < deadline, "checkout provisioning stalled");
            std::thread::sleep(Duration::from_millis(5));
        }
        let placement = state.threads[index].placement.as_ref().unwrap();
        let binding = placement.binding.as_ref().unwrap();
        assert_eq!(binding.state, placement::BindingPhase::Ready);
        assert!(state.threads[index].project.is_some());
        assert_eq!(placement.phase, placement::PlacementPhase::Ready);
        let project = state.project(state.threads[index].project).unwrap();
        let bindings: Vec<_> = state
            .repository_bindings
            .iter()
            .filter(|b| b.repository == binding.repository)
            .collect();
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].state, BindingState::Bound);
        assert_eq!(bindings[0].path, project.path.to_string_lossy().as_ref());
        std::fs::remove_dir_all(state.data_dir).unwrap();
    }
}
