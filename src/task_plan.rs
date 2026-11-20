//! Editable task drafts and dependency-aware launches. A plan node is a
//! draft: preparing or revising it commits only plan state and never
//! allocates a provider. `task_start` turns selected nodes into ordinary
//! delegated tasks through the same target validation, placement and
//! admission path as `delegate_task`, keyed by stable launch receipts.
use super::*;
use crate::protocol::Receipt;
use serde::{Deserialize, Serialize};

pub(crate) const PLAN_ID_MAX: usize = 64;
pub(crate) const NODE_KEY_MAX: usize = 64;
const NODE_TITLE_MAX: usize = 200;
const NODE_BRIEF_MAX: usize = 48 * 1024;
const NODE_DEPS_MAX: usize = 64;
const LAUNCH_KEY_MISMATCH: &str =
    "client_request_id was already used with different launch arguments.";

/// The verified upstream base a stacked launch resolved and committed to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedBase {
    /// The upstream node key the base came from.
    pub node: String,
    #[serde(default)]
    pub branch: Option<String>,
    pub commit: String,
    /// True when the upstream worktree had uncommitted changes at launch; the
    /// base still pins the exact commit, but the dependent cannot see them.
    #[serde(default)]
    pub uncommitted: bool,
}

/// The one-time launch record of a plan node, committed in the same batch as
/// the task allocation it describes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeLaunch {
    /// The outer `task_start` client_request_id that launched this node.
    pub request_key: String,
    /// The derived delegate request key the inner delegation committed under.
    pub delegate_key: String,
    pub task_id: String,
    pub thread_id: usize,
    pub launched_at: u64,
    /// Dependency keys that were unsatisfied at launch under an override.
    #[serde(default)]
    pub overrode: Vec<String>,
    #[serde(default)]
    pub resolved_base: Option<ResolvedBase>,
}

/// One editable draft inside a task plan.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanNode {
    /// Caller-chosen key, unique inside the plan.
    pub key: String,
    pub title: String,
    /// The brief the worker receives before the expected outputs.
    pub brief: String,
    /// What the worker should return for the coordinator's review.
    #[serde(default)]
    pub expected_outputs: Option<String>,
    #[serde(default)]
    pub workspace: Option<Value>,
    #[serde(default)]
    pub target: Option<Value>,
    #[serde(default)]
    pub placement: Option<placement::PlacementSpec>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// The dependency whose verified output commit becomes this node's
    /// worktree base. Must also appear in depends_on.
    #[serde(default)]
    pub stacked_on: Option<String>,
    #[serde(default)]
    pub captain: Option<String>,
    /// CAS revision: `task_update` requires the current value.
    pub revision: u64,
    #[serde(default)]
    pub launch: Option<NodeLaunch>,
    pub created_at: u64,
    pub updated_at: u64,
}

/// A dependency that was not satisfied when a launch ran under an override.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnsatisfiedDep {
    pub node: String,
    /// waiting, failed, cancelled, interrupted or missing.
    pub state: String,
    pub detail: String,
}

/// The durable audit record of an explicit override: what was unsatisfied,
/// who launched anyway and why.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanOverride {
    pub node: String,
    pub unsatisfied: Vec<UnsatisfiedDep>,
    pub reason: String,
    pub caller: usize,
    pub at: u64,
}

/// One coordinator's draft graph. The scope is the durable delegate identity,
/// so a plan survives home resets exactly like delegation receipts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskPlan {
    pub id: String,
    pub scope: String,
    /// Coordinator home thread id when the plan was created; display only.
    pub home: usize,
    #[serde(default)]
    pub captain: Option<usize>,
    #[serde(default)]
    pub nodes: Vec<PlanNode>,
    #[serde(default)]
    pub overrides: Vec<PlanOverride>,
    pub created_at: u64,
    pub updated_at: u64,
}

fn valid_key(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty() && trimmed.len() <= NODE_KEY_MAX && trimmed == value
}

fn required_key(args: &Value, field: &str) -> Result<String, String> {
    let value = args
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{field} is required."))?;
    if !valid_key(value) {
        return Err(format!(
            "{field} must be a non-empty plan or node key of at most {NODE_KEY_MAX} characters, without surrounding whitespace."
        ));
    }
    Ok(value.to_owned())
}

fn plan_id_arg(args: &Value, required: bool) -> Result<Option<String>, String> {
    match args.get("plan") {
        None | Some(Value::Null) if required => Err("plan is required.".into()),
        None | Some(Value::Null) => Ok(None),
        Some(value) => {
            let id = value.as_str().ok_or("plan must be a plan id string.")?;
            if id.trim().is_empty() || id.len() > PLAN_ID_MAX || id.trim() != id {
                return Err(format!(
                    "plan must be a non-empty id of at most {PLAN_ID_MAX} characters, without surrounding whitespace."
                ));
            }
            Ok(Some(id.to_owned()))
        }
    }
}

/// The derived delegate request key for one node launch. Stable across
/// retries and restarts so a replayed or repeated launch can never allocate
/// the node twice.
fn plan_delegate_key(plan_id: &str, node: &str) -> String {
    let identity = json!([plan_id, node]).to_string();
    format!(
        "plan-launch:{}",
        tasks::keyed_command_id("shina.plan-node", &identity)
    )
}

fn launch_command_id(key: &str) -> uuid::Uuid {
    tasks::keyed_command_id("shina.task-launch.request-key", key)
}

/// The keyed launch intent: every launch argument except the key itself.
fn launch_intent(args: &Value) -> Value {
    let mut intent = args.clone();
    if let Some(object) = intent.as_object_mut() {
        object.remove("client_request_id");
    }
    intent
}

fn launch_command(key: &str, args: &Value) -> crate::protocol::Command {
    crate::protocol::Command {
        command_id: launch_command_id(key),
        method: "task_start".into(),
        params: launch_intent(args),
    }
}

fn optional_text(args: &Value, field: &str, max: usize) -> Result<Option<String>, String> {
    match args.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => {
            let text = value
                .as_str()
                .ok_or_else(|| format!("{field} must be a string."))?;
            if text.len() > max {
                return Err(format!("{field} must be at most {max} bytes."));
            }
            Ok(Some(text.to_owned()))
        }
    }
}

/// The delegation-shaped arguments a draft node validates and launches with.
/// A stacked node's verified baseCommit is injected at launch, so it is
/// absent here; stacked placement rules are enforced separately.
fn plan_node_args(node: &PlanNode) -> Value {
    let mut args = json!({
        "prompt": node.brief,
        "title": node.title,
    });
    if let Some(workspace) = &node.workspace {
        args["workspace"] = workspace.clone();
    }
    if let Some(target) = &node.target {
        args["target"] = target.clone();
    }
    if let Some(spec) = &node.placement {
        args["placement"] = serde_json::to_value(spec).unwrap_or(Value::Null);
    }
    if let Some(captain) = &node.captain {
        args["captain"] = Value::from(captain.clone());
    }
    args
}

fn dep_keys(args: &Value, field: &str) -> Result<Vec<String>, String> {
    match args.get(field) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(value) => {
            let items = value
                .as_array()
                .ok_or_else(|| format!("{field} must be an array of node keys."))?;
            if items.len() > NODE_DEPS_MAX {
                return Err(format!("{field} accepts at most {NODE_DEPS_MAX} nodes."));
            }
            let mut keys = Vec::new();
            for item in items {
                let key = item
                    .as_str()
                    .ok_or_else(|| format!("{field} entries must be node keys."))?;
                if !valid_key(key) {
                    return Err(format!(
                        "{field} entries must be non-empty node keys of at most {NODE_KEY_MAX} characters."
                    ));
                }
                if !keys.iter().any(|seen: &String| seen == key) {
                    keys.push(key.to_owned());
                }
            }
            Ok(keys)
        }
    }
}

/// Dependency state reported to callers and used by launch gating. `waiting`
/// covers an unlaunched or still-running upstream; terminal words describe a
/// settled upstream that cannot satisfy the dependency.
fn dep_state_label(launched: bool, task_state: Option<TaskState>) -> &'static str {
    if !launched {
        return "waiting";
    }
    match task_state {
        None => "missing",
        Some(TaskState::Completed) => "satisfied",
        Some(TaskState::Failed) => "failed",
        Some(TaskState::Cancelled) => "cancelled",
        Some(TaskState::Interrupted) => "interrupted",
        _ => "waiting",
    }
}

/// Every dependency edge of a plan must be structurally sound: known nodes,
/// no self-dependency, stacked_on naming a declared dependency and no cycle.
fn validate_plan_graph(plan: &TaskPlan) -> Result<(), String> {
    for node in &plan.nodes {
        if let Some(stacked) = &node.stacked_on {
            if !node.depends_on.iter().any(|dep| dep == stacked) {
                return Err(format!(
                    "Plan node '{}' has stacked_on '{stacked}' without depending on it.",
                    node.key
                ));
            }
        }
        for dep in &node.depends_on {
            if dep == &node.key {
                return Err(format!("Plan node '{}' cannot depend on itself.", node.key));
            }
            if !plan.nodes.iter().any(|candidate| &candidate.key == dep) {
                return Err(format!(
                    "Plan node '{}' depends on '{dep}', which is not in this plan.",
                    node.key
                ));
            }
        }
    }
    // Depth-first cycle detection over depends_on edges.
    let mut state = std::collections::HashMap::new();
    fn visit<'a>(
        plan: &'a TaskPlan,
        key: &'a str,
        state: &mut std::collections::HashMap<&'a str, u8>,
        path: &mut Vec<&'a str>,
    ) -> Result<(), String> {
        match state.get(key) {
            Some(2) => return Ok(()),
            Some(1) => {
                let cycle = path
                    .iter()
                    .position(|entry| *entry == key)
                    .map(|at| {
                        path[at..]
                            .iter()
                            .chain(std::iter::once(&key))
                            .copied()
                            .collect::<Vec<_>>()
                            .join(" -> ")
                    })
                    .unwrap_or_else(|| key.to_owned());
                return Err(format!("The task plan has a dependency cycle: {cycle}."));
            }
            _ => {}
        }
        state.insert(key, 1);
        path.push(key);
        if let Some(node) = plan.nodes.iter().find(|node| node.key == key) {
            for dep in &node.depends_on {
                visit(plan, dep, state, path)?;
            }
        }
        path.pop();
        state.insert(key, 2);
        Ok(())
    }
    let mut path = Vec::new();
    for node in &plan.nodes {
        visit(plan, &node.key, &mut state, &mut path)?;
    }
    Ok(())
}

impl State {
    /// Plan tools coordinate new work like delegate_task: workers and earlier
    /// Captain homes may read but never write plan state.
    pub(super) fn task_plan_write_guard(&self, caller_index: usize) -> Result<(), String> {
        if self.threads[caller_index].delegated {
            return Err(
                "Delegated threads cannot manage task plans. Do the work in this thread.".into(),
            );
        }
        if self.threads[caller_index].captain_home
            && self.effective_home(self.threads[caller_index].id) != self.threads[caller_index].id
        {
            return Err(
                "Open the captain's primary home to manage task plans. This is an earlier conversation."
                    .into(),
            );
        }
        Ok(())
    }

    fn task_plan_index(&self, scope: &str, id: &str) -> Option<usize> {
        self.task_plans
            .iter()
            .position(|plan| plan.id == id && plan.scope == scope)
    }

    /// The dependency report for one edge. An override covering this edge
    /// reports `overridden` and still counts as satisfied for launch gating.
    fn dep_report(&self, plan: &TaskPlan, node: &PlanNode, dep: &str) -> Value {
        let dep_node = plan.nodes.iter().find(|candidate| &candidate.key == dep);
        let (state, detail, task_id, thread_id) = match dep_node.and_then(|n| n.launch.as_ref()) {
            None => (
                "waiting",
                format!("node '{dep}' has not launched"),
                None,
                None,
            ),
            Some(launch) => match self.tasks.iter().find(|task| task.id == launch.task_id) {
                None => (
                    "missing",
                    format!("node '{dep}' launched but its task record is missing"),
                    Some(launch.task_id.clone()),
                    Some(launch.thread_id),
                ),
                Some(task) => {
                    let state = dep_state_label(true, Some(task.state));
                    let detail = match task.state {
                        TaskState::Completed => {
                            format!("node '{dep}' completed")
                        }
                        TaskState::Failed => format!(
                            "node '{dep}' failed{}",
                            if task.error_excerpt.is_empty() {
                                String::new()
                            } else {
                                format!(": {}", truncate(&task.error_excerpt, 300))
                            }
                        ),
                        TaskState::Cancelled => format!("node '{dep}' was cancelled"),
                        TaskState::Interrupted => {
                            format!("node '{dep}' is interrupted and needs attention")
                        }
                        _ => format!("node '{dep}' is still running"),
                    };
                    (
                        state,
                        detail,
                        Some(launch.task_id.clone()),
                        Some(launch.thread_id),
                    )
                }
            },
        };
        let overridden = plan.overrides.iter().any(|entry| {
            entry.node == node.key
                && entry
                    .unsatisfied
                    .iter()
                    .any(|dep_item| dep_item.node == dep)
        });
        json!({
            "node": dep,
            "state": if overridden { "overridden" } else { state },
            "satisfied": overridden || state == "satisfied",
            "detail": detail,
            "task_id": task_id,
            "thread_id": thread_id,
        })
    }

    /// Unsatisfied dependency edges of a draft node, ignoring edges already
    /// covered by a recorded override.
    fn unsatisfied_deps(&self, plan: &TaskPlan, node: &PlanNode) -> Vec<UnsatisfiedDep> {
        node.depends_on
            .iter()
            .filter(|dep| {
                !plan.overrides.iter().any(|entry| {
                    entry.node == node.key
                        && entry.unsatisfied.iter().any(|item| &item.node == *dep)
                })
            })
            .filter_map(|dep| {
                let report = self.dep_report(plan, node, dep);
                (report["state"] != "satisfied").then(|| UnsatisfiedDep {
                    node: dep.clone(),
                    state: report["state"].as_str().unwrap_or("waiting").to_owned(),
                    detail: report["detail"].as_str().unwrap_or_default().to_owned(),
                })
            })
            .collect()
    }

    /// The plan state the MCP API and the index projection publish. Node
    /// dependency state is computed live so a stopped or released upstream is
    /// never silently treated as satisfied.
    pub(super) fn task_plan_value(&self, plan: &TaskPlan) -> Value {
        let nodes: Vec<Value> = plan
            .nodes
            .iter()
            .map(|node| {
                let dependencies: Vec<Value> = node
                    .depends_on
                    .iter()
                    .map(|dep| self.dep_report(plan, node, dep))
                    .collect();
                let launched = node.launch.is_some();
                let blocked = !launched
                    && dependencies.iter().any(|dep| {
                        matches!(
                            dep["state"].as_str(),
                            Some("failed" | "cancelled" | "interrupted" | "missing")
                        )
                    });
                let ready = !launched
                    && !blocked
                    && dependencies
                        .iter()
                        .all(|dep| dep["satisfied"] == true);
                let launch = node.launch.as_ref().map(|launch| {
                    let task = self
                        .tasks
                        .iter()
                        .find(|task| task.id == launch.task_id);
                    json!({
                        "task_id": launch.task_id,
                        "thread_id": launch.thread_id,
                        "launched_at": launch.launched_at,
                        "task_state": task.map(|task| task.state.id()),
                        "overrode": launch.overrode,
                        "resolved_base": launch.resolved_base,
                        "request_key": launch.request_key,
                    })
                });
                json!({
                    "key": node.key,
                    "title": node.title,
                    "brief": node.brief,
                    "expected_outputs": node.expected_outputs,
                    "workspace": node.workspace,
                    "target": node.target,
                    "placement": node.placement,
                    "depends_on": node.depends_on,
                    "stacked_on": node.stacked_on,
                    "captain": node.captain,
                    "revision": node.revision,
                    "status": if launched { "launched" } else if blocked { "blocked" } else if ready { "ready" } else { "draft" },
                    "dependencies": dependencies,
                    "launch": launch,
                    "created_at": node.created_at,
                    "updated_at": node.updated_at,
                })
            })
            .collect();
        json!({
            "id": plan.id,
            "home": plan.home,
            "captain": plan.captain,
            "nodes": nodes,
            "overrides": plan.overrides,
            "created_at": plan.created_at,
            "updated_at": plan.updated_at,
        })
    }

    /// Draft spec validation shared by prepare and update: target, workspace
    /// and placement parse exactly like a delegation's, and a stacked node
    /// may not name its own base or a remote machine.
    fn validate_node_target(
        &self,
        caller_index: usize,
        args: &Value,
        stacked: bool,
    ) -> Result<(), String> {
        parse_delegation_target(args)?;
        let remote = remote_tasks::target_machine(args)?.is_some_and(|machine| machine != "local");
        if remote {
            return Err(if stacked {
                "A stacked node must run on the local machine to share the upstream repository. Remove target.machine or stacked_on."
            } else {
                "Task plans currently start on the local machine only. Use delegate_task for a linked executor."
            }.into());
        }
        if let Some(spec) = placement::spec_from_args(args)? {
            let requested = placement::validate_spec(&spec)?;
            if stacked {
                if spec.base_branch.is_some()
                    || spec.base_commit.is_some()
                    || spec.upstream_thread_id.is_some()
                    || spec.start_from_origin.is_some()
                {
                    return Err(
                        "A stacked node's base comes from its upstream commit; do not also set placement.baseBranch, baseCommit, upstreamThreadId or startFromOrigin."
                            .into(),
                    );
                }
                if matches!(
                    requested,
                    Some(placement::PlacementMode::Shared | placement::PlacementMode::Existing)
                ) {
                    return Err(
                        "A stacked node needs an isolated worktree on the upstream commit; mode shared or existing cannot take a base."
                            .into(),
                    );
                }
            }
        }
        self.delegation_workspace(caller_index, args)?;
        Ok(())
    }

    /// Build the delegate_task arguments a node's launch replays through the
    /// ordinary allocation path. The derived request key makes the inner
    /// delegation idempotent on its own.
    fn plan_delegate_args(
        &self,
        plan: &TaskPlan,
        node: &PlanNode,
        resolved_base: Option<&ResolvedBase>,
    ) -> Value {
        let brief = match &node.expected_outputs {
            Some(outputs) if !outputs.trim().is_empty() => {
                format!("{}\n\nExpected outputs:\n{outputs}", node.brief)
            }
            _ => node.brief.clone(),
        };
        let mut args = json!({
            "prompt": brief,
            "title": node.title,
            "client_request_id": plan_delegate_key(&plan.id, &node.key),
        });
        if let Some(workspace) = &node.workspace {
            args["workspace"] = workspace.clone();
        }
        if let Some(target) = &node.target {
            args["target"] = target.clone();
        }
        if let Some(captain) = &node.captain {
            args["captain"] = Value::from(captain.clone());
        }
        if let Some(base) = resolved_base {
            let mut placement = json!({"mode": "isolated", "baseCommit": base.commit});
            if let Some(branch) = node
                .placement
                .as_ref()
                .and_then(|spec| spec.branch.as_ref())
            {
                placement["branch"] = Value::from(branch.clone());
            }
            args["placement"] = placement;
        } else if let Some(spec) = &node.placement {
            args["placement"] = serde_json::to_value(spec).unwrap_or(Value::Null);
        }
        args
    }

    /// Resolve the exact upstream output commit a stacked node bases on. The
    /// upstream task must be completed, local and provisioned in a worktree;
    /// its HEAD is verified with git before it becomes the child's base.
    fn resolve_stacked_base(
        &self,
        plan: &TaskPlan,
        node: &PlanNode,
        ctx: &crate::vcs::GitContext,
    ) -> Result<ResolvedBase, String> {
        let dep = node.stacked_on.as_ref().ok_or("node is not stacked")?;
        let dep_node = plan
            .nodes
            .iter()
            .find(|candidate| &candidate.key == dep)
            .ok_or_else(|| format!("stacked_on '{dep}' is not in this plan"))?;
        let launch = dep_node
            .launch
            .as_ref()
            .ok_or_else(|| format!("stacked_on '{dep}' has not launched"))?;
        let task = self
            .tasks
            .iter()
            .find(|task| task.id == launch.task_id)
            .ok_or_else(|| format!("stacked_on '{dep}' launched but its task record is missing"))?;
        if task.remote.is_some() {
            return Err(format!(
                "Cannot stack on '{dep}': remote work has no local worktree commit to base on."
            ));
        }
        if task.state != TaskState::Completed {
            return Err(format!(
                "Cannot stack on '{dep}': its task is {}, not completed.",
                task.state.id()
            ));
        }
        let thread = self
            .threads
            .iter()
            .find(|thread| thread.id == task.child_thread_id)
            .ok_or_else(|| format!("Cannot stack on '{dep}': its worker thread is gone."))?;
        let checkout = thread
            .worktree_path
            .as_ref()
            .filter(|path| !path.is_empty())
            .ok_or_else(|| {
                format!(
                    "Cannot stack on '{dep}': it ran in a shared checkout with no worktree commit to base on."
                )
            })?;
        let branch = thread
            .placement
            .as_ref()
            .and_then(|placement| placement.branch.clone());
        let head = crate::vcs::run_git_checked(
            "TaskPlan.upstreamCommit",
            std::path::Path::new(checkout),
            &["rev-parse", "--verify", "--quiet", "HEAD^{commit}"],
            crate::vcs::SHORT_TIMEOUT,
            "The upstream worktree has no resolvable HEAD commit to stack on.",
            ctx,
        )?;
        let commit = head.stdout.trim().to_owned();
        let uncommitted = crate::vcs::run_git_raw(
            "TaskPlan.upstreamDirty",
            std::path::Path::new(checkout),
            &["status", "--porcelain"],
            crate::vcs::SHORT_TIMEOUT,
            ctx,
        )
        .map(|out| out.exit_code == Some(0) && !out.stdout.trim().is_empty())
        .unwrap_or(false);
        Ok(ResolvedBase {
            node: dep.clone(),
            branch,
            commit,
            uncommitted,
        })
    }

    /// Coordinator capacity check for a multi-node start: the whole batch is
    /// refused before any launch when it would exceed the unfinished-task
    /// limit, so a capacity refusal never leaves a half-launched request.
    fn check_plan_capacity(&self, caller: usize, additional: usize) -> Result<(), String> {
        let Some(limit) = self.scheduler.limits.coordinator_unfinished else {
            return Ok(());
        };
        let home = Some(self.effective_home(caller));
        let current = self
            .tasks
            .iter()
            .filter(|task| self.task_home(task) == home && !task.state.finished())
            .count();
        if current + additional > limit {
            return Err(format!(
                "Starting {additional} plan nodes would exceed this coordinator's {limit} unfinished tasks (currently {current}). Finish or dismiss tasks first."
            ));
        }
        Ok(())
    }

    /// `task_prepare`: create a plan (when `plan` is absent) or add a draft
    /// node to a plan the caller owns. Commits plan state only; no task,
    /// thread or provider is allocated.
    pub(super) fn task_plan_prepare(
        &mut self,
        caller_index: usize,
        args: &Value,
    ) -> Result<Value, String> {
        self.task_plan_write_guard(caller_index)?;
        let scope = self.delegate_scope_device(caller_index);
        let now = lifecycle::now();
        let new_plan = |state: &mut Self, id: String| {
            let home = state.caller_delegate_home(caller_index);
            let captain = state.threads[caller_index]
                .captain
                .filter(|_| state.threads[caller_index].captain_home);
            state.task_plans.push(TaskPlan {
                id,
                scope: scope.clone(),
                home,
                captain,
                nodes: Vec::new(),
                overrides: Vec::new(),
                created_at: now,
                updated_at: now,
            });
            state.task_plans.len() - 1
        };
        let (plan_index, created) = match plan_id_arg(args, false)? {
            Some(id) => match self.task_plan_index(&scope, &id) {
                Some(index) => (index, false),
                None => (new_plan(self, id), true),
            },
            None => (new_plan(self, format!("plan-{}", tasks::unique_id())), true),
        };
        let result = self.task_plan_create_node(plan_index, caller_index, args);
        if created && result.is_err() {
            self.task_plans.pop();
        }
        result
    }

    fn task_plan_create_node(
        &mut self,
        plan_index: usize,
        caller_index: usize,
        args: &Value,
    ) -> Result<Value, String> {
        let node_key = required_key(args, "node")?;
        let brief = optional_text(args, "prompt", NODE_BRIEF_MAX)?
            .map(|text| text.trim().to_owned())
            .filter(|text| !text.is_empty())
            .ok_or("prompt must describe the task.")?;
        let title = optional_text(args, "title", NODE_TITLE_MAX)?
            .map(|text| text.trim().to_owned())
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| brief.chars().take(45).collect());
        let expected_outputs = optional_text(args, "expected_outputs", NODE_BRIEF_MAX)?;
        let workspace = args.get("workspace").cloned().filter(|v| !v.is_null());
        let target = args.get("target").cloned().filter(|v| !v.is_null());
        let placement = placement::spec_from_args(args)?;
        let depends_on = dep_keys(args, "depends_on")?;
        let stacked_on = match args.get("stacked_on") {
            None | Some(Value::Null) => None,
            Some(_) => Some(required_key(args, "stacked_on")?),
        };
        let captain = optional_text(args, "captain", 128)?
            .map(|text| text.trim().to_owned())
            .filter(|text| !text.is_empty());
        if self.task_plans[plan_index]
            .nodes
            .iter()
            .any(|node| node.key == node_key)
        {
            return Err(format!(
                "Plan '{}' already has a node '{node_key}'.",
                self.task_plans[plan_index].id
            ));
        }
        // Validate dependencies against the plan with this node conceptually
        // present, then validate the draft's target and placement shape.
        let now = lifecycle::now();
        let node = PlanNode {
            key: node_key.clone(),
            title,
            brief,
            expected_outputs,
            workspace,
            target,
            placement,
            depends_on,
            stacked_on,
            captain,
            revision: 1,
            launch: None,
            created_at: now,
            updated_at: now,
        };
        self.task_plans[plan_index].nodes.push(node);
        let validation = (|| {
            validate_plan_graph(&self.task_plans[plan_index])?;
            let node = self.task_plans[plan_index]
                .nodes
                .last()
                .expect("node pushed");
            self.validate_node_target(caller_index, args, node.stacked_on.is_some())?;
            // The same spec delegate_task validates, so a draft that could
            // never launch is refused while it is still a draft.
            self.local_delegate_spec(caller_index, args)
                .map_err(|error| format!("Plan node '{node_key}': {error}"))
        })();
        if let Err(error) = validation {
            self.task_plans[plan_index].nodes.pop();
            return Err(error);
        }
        self.task_plans[plan_index].updated_at = now;
        self.checkpoint();
        let plan = &self.task_plans[plan_index];
        Ok(json!({
            "plan_id": plan.id,
            "node": plan.nodes.last().map(|node| node.key.clone()),
            "revision": 1,
            "plan": self.task_plan_value(plan),
        }))
    }

    /// `task_update`: revise a draft under a required CAS revision. A launched
    /// node is frozen: edits can never change a running attempt.
    pub(super) fn task_plan_update(
        &mut self,
        caller_index: usize,
        args: &Value,
    ) -> Result<Value, String> {
        self.task_plan_write_guard(caller_index)?;
        let scope = self.delegate_scope_device(caller_index);
        let plan_id = plan_id_arg(args, true)?.expect("required");
        let node_key = required_key(args, "node")?;
        let expected = args["expected_revision"]
            .as_u64()
            .ok_or("expected_revision is required.")?;
        let plan_index = self
            .task_plan_index(&scope, &plan_id)
            .ok_or_else(|| format!("Unknown task plan '{plan_id}'."))?;
        let node_index = self.task_plans[plan_index]
            .nodes
            .iter()
            .position(|node| node.key == node_key)
            .ok_or_else(|| format!("Plan '{plan_id}' has no node '{node_key}'."))?;
        if self.task_plans[plan_index].nodes[node_index]
            .launch
            .is_some()
        {
            return Err(format!(
                "Plan node '{node_key}' already launched; its attempt cannot be edited."
            ));
        }
        let current = self.task_plans[plan_index].nodes[node_index].revision;
        if current != expected {
            return Err(format!(
                "Plan node '{node_key}' changed since revision {expected}. Read task_plan again and preserve intervening changes."
            ));
        }
        // Apply provided fields to a scratch copy so a failed edit leaves the
        // committed revision untouched.
        let mut next = self.task_plans[plan_index].nodes[node_index].clone();
        if let Some(value) = args.get("prompt") {
            let brief = value
                .as_str()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .filter(|text| text.len() <= NODE_BRIEF_MAX)
                .ok_or("prompt must describe the task.")?;
            next.brief = brief.to_owned();
        }
        if let Some(value) = args.get("title") {
            let title = value
                .as_str()
                .map(str::trim)
                .filter(|text| !text.is_empty() && text.len() <= NODE_TITLE_MAX)
                .ok_or("title must be a non-empty string of at most 200 bytes.")?;
            next.title = title.to_owned();
        }
        if let Some(value) = args.get("expected_outputs") {
            next.expected_outputs = if value.is_null() {
                None
            } else {
                Some(
                    value
                        .as_str()
                        .filter(|text| text.len() <= NODE_BRIEF_MAX)
                        .ok_or("expected_outputs must be a string of at most 48 KiB.")?
                        .to_owned(),
                )
            };
        }
        if let Some(value) = args.get("workspace") {
            next.workspace = if value.is_null() {
                None
            } else {
                Some(value.clone())
            };
        }
        if let Some(value) = args.get("target") {
            next.target = if value.is_null() {
                None
            } else {
                Some(value.clone())
            };
        }
        if let Some(value) = args.get("placement") {
            next.placement = if value.is_null() {
                None
            } else {
                Some(
                    serde_json::from_value::<placement::PlacementSpec>(value.clone())
                        .map_err(|error| format!("Invalid placement: {error}"))?,
                )
            };
        }
        if let Some(value) = args.get("depends_on") {
            next.depends_on = dep_keys(&json!({"depends_on": value}), "depends_on")?;
        }
        if let Some(value) = args.get("stacked_on") {
            next.stacked_on = if value.is_null() {
                None
            } else {
                Some(
                    value
                        .as_str()
                        .filter(|key| valid_key(key))
                        .ok_or("stacked_on must be a node key.")?
                        .to_owned(),
                )
            };
        }
        if let Some(value) = args.get("captain") {
            next.captain = if value.is_null() {
                None
            } else {
                Some(
                    value
                        .as_str()
                        .map(str::trim)
                        .filter(|text| !text.is_empty())
                        .ok_or("captain must be a non-empty string.")?
                        .to_owned(),
                )
            };
        }
        let mut candidate = self.task_plans[plan_index].clone();
        candidate.nodes[node_index] = next.clone();
        validate_plan_graph(&candidate)?;
        let merged_args = plan_node_args(&next);
        self.validate_node_target(caller_index, &merged_args, next.stacked_on.is_some())?;
        self.local_delegate_spec(caller_index, &merged_args)
            .map_err(|error| format!("Plan node '{node_key}': {error}"))?;
        let now = lifecycle::now();
        let node = &mut self.task_plans[plan_index].nodes[node_index];
        *node = next;
        node.revision = expected + 1;
        node.updated_at = now;
        self.task_plans[plan_index].updated_at = now;
        self.checkpoint();
        let plan = &self.task_plans[plan_index];
        Ok(json!({
            "plan_id": plan.id,
            "node": node_key,
            "revision": expected + 1,
            "plan": self.task_plan_value(plan),
        }))
    }

    /// `task_start`: launch selected draft nodes that are ready. Validation of
    /// every requested node, its dependencies, target and base finishes before
    /// any launch, and the keyed receipt commits before the scheduler can
    /// deliver a provider effect.
    pub(super) fn task_plan_start(
        &mut self,
        caller_index: usize,
        args: &Value,
    ) -> Result<Value, String> {
        self.task_plan_write_guard(caller_index)?;
        let caller = self.threads[caller_index].id;
        let scope = self.delegate_scope_device(caller_index);
        let key = args["client_request_id"]
            .as_str()
            .map(str::trim)
            .filter(|key| !key.is_empty() && key.len() <= 256)
            .ok_or(
                "task_start requires client_request_id: a nonempty caller-chosen string of at most 256 characters. Reuse it verbatim to retry; never reuse it for different arguments.",
            )?;
        let plan_id = plan_id_arg(args, true)?.expect("required");
        let mut requested = dep_keys(args, "nodes")?;
        if let Some(single) = args.get("node").filter(|v| !v.is_null()) {
            let single = single
                .as_str()
                .filter(|key| valid_key(key))
                .ok_or("node must be a node key.")?;
            if !requested.iter().any(|key| key == single) {
                requested.push(single.to_owned());
            }
        }
        if requested.is_empty() {
            return Err("task_start requires node or nodes naming draft nodes to launch.".into());
        }
        // An identical retry replays the committed launch receipt before any
        // state is rechecked, exactly like keyed sends and cancels.
        let intent = launch_command(key, args).hash();
        if let Some(replayed) = self.task_launch_receipt_replay(caller_index, key, &intent)? {
            return Ok(replayed);
        }
        self.delegate_receipt_ready()?;
        let plan_index = self
            .task_plan_index(&scope, &plan_id)
            .ok_or_else(|| format!("Unknown task plan '{plan_id}'."))?;
        validate_plan_graph(&self.task_plans[plan_index])?;
        // Classify requested nodes. A node already launched under this same
        // request key is a replay include; under any other key it refuses.
        let mut to_launch: Vec<String> = Vec::new();
        let mut replayed: Vec<String> = Vec::new();
        for node_key in &requested {
            let node = self.task_plans[plan_index]
                .nodes
                .iter()
                .find(|node| &node.key == node_key)
                .ok_or_else(|| format!("Plan '{plan_id}' has no node '{node_key}'."))?;
            match &node.launch {
                Some(launch) if launch.request_key == key => replayed.push(node_key.clone()),
                Some(_) => {
                    return Err(format!(
                        "Plan node '{node_key}' already launched and cannot be launched again."
                    ));
                }
                None => to_launch.push(node_key.clone()),
            }
        }
        // Dependency gating: only satisfied or explicitly overridden edges
        // allow a launch. Failures name every unsatisfied edge so the caller
        // sees exactly what blocks each node.
        let override_reason = args["override_reason"]
            .as_str()
            .map(str::trim)
            .filter(|reason| !reason.is_empty())
            .map(str::to_owned);
        let mut blocked: Vec<String> = Vec::new();
        let mut overrides: Vec<(String, Vec<UnsatisfiedDep>)> = Vec::new();
        for node_key in &to_launch {
            let (node_index, unsatisfied) = {
                let plan = &self.task_plans[plan_index];
                let node_index = plan
                    .nodes
                    .iter()
                    .position(|node| &node.key == node_key)
                    .expect("checked above");
                (
                    node_index,
                    self.unsatisfied_deps(plan, &plan.nodes[node_index]),
                )
            };
            let _ = node_index;
            if unsatisfied.is_empty() {
                continue;
            }
            if override_reason.is_some() {
                overrides.push((node_key.clone(), unsatisfied));
            } else {
                let detail = unsatisfied
                    .iter()
                    .map(|dep| format!("{}", dep.detail))
                    .collect::<Vec<_>>()
                    .join("; ");
                blocked.push(format!("node '{node_key}' is not ready: {detail}"));
            }
        }
        if !blocked.is_empty() {
            return Err(format!(
                "{}. Pass override_reason to launch anyway; the override is recorded on the plan.",
                blocked.join("; ")
            ));
        }
        // Pre-validate every launch's target, workspace and stacked base
        // before any node is allocated, so an invalid node never leaves a
        // half-launched request.
        #[cfg(test)]
        let git_ctx = self.test_git_context.clone().unwrap_or_default();
        #[cfg(not(test))]
        let git_ctx = crate::vcs::GitContext::default();
        let mut prepared = Vec::new();
        let mut additional = 0usize;
        for node_key in &to_launch {
            let (node, overrode) = {
                let plan = &self.task_plans[plan_index];
                let node = plan
                    .nodes
                    .iter()
                    .find(|node| &node.key == node_key)
                    .expect("checked above")
                    .clone();
                let overrode = overrides
                    .iter()
                    .find(|(key, _)| key == node_key)
                    .map(|(_, deps)| deps.iter().map(|dep| dep.node.clone()).collect())
                    .unwrap_or_default();
                (node, overrode)
            };
            let resolved = if node.stacked_on.is_some() {
                if remote_tasks::target_machine(&json!({"target": node.target}))?
                    .is_some_and(|machine| machine != "local")
                {
                    return Err(format!(
                        "Plan node '{node_key}' is stacked and must run on the local machine."
                    ));
                }
                Some(self.resolve_stacked_base(&self.task_plans[plan_index], &node, &git_ctx)?)
            } else {
                None
            };
            let delegate_args =
                self.plan_delegate_args(&self.task_plans[plan_index], &node, resolved.as_ref());
            let delegate_key = plan_delegate_key(&plan_id, &node.key);
            let staged =
                match self.delegate_receipt_replay(caller_index, &delegate_key, &delegate_args)? {
                    Some(receipt) => Err(receipt),
                    None => {
                        let id = self
                            .next_id
                            .checked_add(additional)
                            .ok_or("Thread IDs exhausted.")?;
                        let launch = self
                            .prepare_local_delegation(
                                caller_index,
                                &delegate_args,
                                Some(&delegate_key),
                                id,
                            )
                            .map_err(|error| format!("Plan node '{node_key}': {error}"))?;
                        additional += 1;
                        Ok(launch)
                    }
                };
            prepared.push((node_key.clone(), delegate_args, resolved, overrode, staged));
        }
        self.check_plan_capacity(caller, additional)?;
        self.scheduler.check_pending_additional(additional)?;
        // Commit the audited override before the first launch effect.
        let now = lifecycle::now();
        for (node_key, unsatisfied) in &overrides {
            self.task_plans[plan_index].overrides.push(PlanOverride {
                node: node_key.clone(),
                unsatisfied: unsatisfied.clone(),
                reason: override_reason.clone().unwrap_or_default(),
                caller,
                at: now,
            });
        }
        // Launch each prepared node through the ordinary delegation path; the
        // inner keyed receipt commits before its provider effect, and the
        // derived request key makes a replay impossible to double-allocate.
        let mut started: Vec<Value> = Vec::new();
        for (node_key, delegate_args, resolved, overrode, staged) in prepared {
            let delegate_key = plan_delegate_key(&plan_id, &node_key);
            // If a crash committed the inner delegation but not the launch
            // record, the derived key replays its original task instead of
            // allocating a duplicate.
            let status = match staged {
                Err(replayed) => replayed,
                Ok(launch) => self.stage_local_delegation(
                    caller_index,
                    &delegate_args,
                    Some(&delegate_key),
                    launch,
                )?,
            };
            let task_id = status["task_id"]
                .as_str()
                .ok_or("The launched node returned no task_id.")?
                .to_owned();
            let thread_id = status["thread_id"]
                .as_u64()
                .ok_or("The launched node returned no thread_id.")?
                as usize;
            if let Some(task) = self.tasks.iter_mut().find(|task| task.id == task_id) {
                task.plan_node = Some(tasks::PlanTaskRef {
                    plan: plan_id.clone(),
                    node: node_key.clone(),
                });
            }
            let plan = &mut self.task_plans[plan_index];
            if let Some(node) = plan.nodes.iter_mut().find(|node| node.key == node_key) {
                node.launch = Some(NodeLaunch {
                    request_key: key.to_owned(),
                    delegate_key: plan_delegate_key(&plan_id, &node_key),
                    task_id: task_id.clone(),
                    thread_id,
                    launched_at: lifecycle::now(),
                    overrode,
                    resolved_base: resolved,
                });
                node.updated_at = lifecycle::now();
            }
            plan.updated_at = lifecycle::now();
            started.push(json!({
                "node": node_key,
                "task_id": task_id,
                "thread_id": thread_id,
                "status": status["status"],
                "queue_position": status["queue_position"],
                "replayed": status["replayed"] == true,
            }));
        }
        for node_key in &replayed {
            let (task_id, thread_id) = {
                let plan = &self.task_plans[plan_index];
                let launch = plan
                    .nodes
                    .iter()
                    .find(|node| &node.key == node_key)
                    .and_then(|node| node.launch.as_ref())
                    .expect("replayed node launched");
                (launch.task_id.clone(), launch.thread_id)
            };
            let status = self.mcp_call(caller, "task_status", &json!({"thread_id": thread_id}))?;
            started.push(json!({
                "node": node_key,
                "task_id": task_id,
                "thread_id": thread_id,
                "status": status["status"],
                "queue_position": status["queue_position"],
                "replayed": true,
            }));
        }
        let mut result = json!({
            "plan_id": plan_id,
            "started": started,
            "plan": self.task_plan_value(&self.task_plans[plan_index]),
        });
        self.record_task_launch_receipt(caller_index, key, args, &result)?;
        result["requestKey"] = json!(key);
        self.checkpoint();
        self.start_queued_tasks();
        Ok(result)
    }

    /// `task_plan`: read one plan or every plan in the caller's scope with
    /// live dependency and launch state.
    pub(super) fn task_plan_read(
        &mut self,
        caller_index: usize,
        args: &Value,
    ) -> Result<Value, String> {
        let scope = self.delegate_scope_device(caller_index);
        match plan_id_arg(args, false)? {
            Some(id) => {
                let plan = self
                    .task_plans
                    .iter()
                    .find(|plan| plan.id == id && plan.scope == scope)
                    .ok_or_else(|| format!("Unknown task plan '{id}'."))?;
                Ok(json!({
                    "plan_id": plan.id,
                    "plan": self.task_plan_value(plan),
                }))
            }
            None => Ok(self.task_plans_view(caller_index)),
        }
    }

    /// Every plan in the caller's delegate scope, exactly as `task_plan` without
    /// a plan id returns it. The desktop panel renders this same value.
    pub(crate) fn task_plans_view(&self, caller_index: usize) -> Value {
        let scope = self.delegate_scope_device(caller_index);
        json!({
            "plans": self
                .task_plans
                .iter()
                .filter(|plan| plan.scope == scope)
                .map(|plan| self.task_plan_value(plan))
                .collect::<Vec<_>>(),
        })
    }

    /// The launch receipt commits in the same batch as the launched nodes, so
    /// a crash after commit but before the reply can never start them twice.
    fn task_launch_receipt_replay(
        &mut self,
        caller_index: usize,
        key: &str,
        intent: &str,
    ) -> Result<Option<Value>, String> {
        let scope = self.delegate_scope_device(caller_index);
        let id = launch_command_id(key);
        let receipt =
            if let Some(database) = crate::database::for_path(&self.data_dir.join("index.json")) {
                database.delegate_tool_receipt(&scope, id)?
            } else {
                self.command_receipt(&scope, id).cloned()
            };
        let Some(receipt) = receipt else {
            return Ok(None);
        };
        if receipt.hash != intent {
            return Err(LAUNCH_KEY_MISMATCH.into());
        }
        let mut result = receipt.result;
        result["replayed"] = json!(true);
        result["plan"] = self
            .task_plan_index(&scope, result["plan_id"].as_str().unwrap_or_default())
            .map(|index| self.task_plan_value(&self.task_plans[index]))
            .unwrap_or(Value::Null);
        Ok(Some(result))
    }

    fn record_task_launch_receipt(
        &mut self,
        caller_index: usize,
        key: &str,
        args: &Value,
        result: &Value,
    ) -> Result<(), String> {
        let scope = self.delegate_scope_device(caller_index);
        let command = launch_command(key, args);
        if let Some(database) = crate::database::for_path(&self.data_dir.join("index.json")) {
            return database
                .stage_task_launch_receipt(&scope, &command, result.clone())
                .map_err(|error| self.fail_delegate_receipt(error));
        }
        let hash = command.hash();
        if let Some(existing) = self
            .command_receipts
            .iter()
            .find(|receipt| receipt.device_id == scope && receipt.command_id == command.command_id)
        {
            return if existing.hash == hash {
                Ok(())
            } else {
                Err(LAUNCH_KEY_MISMATCH.into())
            };
        }
        self.command_receipts.push(Receipt {
            command_id: command.command_id,
            device_id: scope,
            hash,
            status: crate::protocol::ReceiptStatus::Accepted,
            result: result.clone(),
            seq: self.server_seq,
        });
        self.persist_receipt_snapshot()?;
        Ok(())
    }

    /// Restore persisted plans from the index, with structural validation in
    /// the same spirit as restored remote tasks.
    pub(super) fn restore_task_plans(&mut self, index: &Value) -> Result<(), String> {
        let invalid = |why: &str| format!("The saved index contains an invalid task plan: {why}.");
        let mut plans: Vec<TaskPlan> = match index.get("taskPlans") {
            None | Some(Value::Null) => Vec::new(),
            Some(value) => serde_json::from_value(value.clone())
                .map_err(|error| invalid(&format!("{error}")))?,
        };
        let mut ids = std::collections::HashSet::new();
        for plan in &mut plans {
            if plan.id.is_empty()
                || plan.id.len() > PLAN_ID_MAX
                || plan.scope.is_empty()
                || !ids.insert((plan.scope.clone(), plan.id.clone()))
            {
                return Err(invalid("duplicate or malformed plan id"));
            }
            let mut keys = std::collections::HashSet::new();
            for node in &mut plan.nodes {
                if !valid_key(&node.key)
                    || node.revision == 0
                    || node.brief.len() > NODE_BRIEF_MAX
                    || node.depends_on.len() > NODE_DEPS_MAX
                    || !keys.insert(node.key.clone())
                {
                    return Err(invalid("malformed node"));
                }
                if let Some(launch) = &node.launch {
                    if launch.task_id.is_empty()
                        || launch.request_key.is_empty()
                        || launch.delegate_key != plan_delegate_key(&plan.id, &node.key)
                    {
                        return Err(invalid("malformed launch record"));
                    }
                }
            }
            validate_plan_graph(plan).map_err(|error| invalid(&error))?;
        }
        self.task_plans = plans;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo() -> State {
        let mut state = State::new(PathBuf::new(), false);
        state.demo = true;
        state.use_pi_for_test();
        state
    }

    fn prepare(state: &mut State, node: &str, extra: Value) -> Result<Value, String> {
        let mut args = json!({"plan": "p", "node": node, "prompt": format!("Work on {node}")});
        if let Value::Object(extra) = extra {
            args.as_object_mut().unwrap().extend(extra);
        }
        state.mcp_call(1, "task_prepare", &args)
    }

    fn update(state: &mut State, node: &str, revision: u64, extra: Value) -> Result<Value, String> {
        let mut args = json!({"plan": "p", "node": node, "expected_revision": revision});
        if let Value::Object(extra) = extra {
            args.as_object_mut().unwrap().extend(extra);
        }
        state.mcp_call(1, "task_update", &args)
    }

    fn start(state: &mut State, args: Value) -> Result<Value, String> {
        state.mcp_call(1, "task_start", &args)
    }

    fn node_view<'a>(state: &'a State, key: &str) -> Value {
        let view = state
            .task_plan_value(&state.task_plans[0])
            .as_object()
            .unwrap()
            .clone();
        view["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["key"] == key)
            .expect("node in view")
            .clone()
    }

    fn finish_task(state: &mut State, node: &str, outcome: TaskState) {
        let launch = state.task_plans[0]
            .nodes
            .iter()
            .find(|entry| entry.key == node)
            .and_then(|node| node.launch.as_ref())
            .expect("launched");
        let index = state
            .tasks
            .iter()
            .position(|task| task.id == launch.task_id)
            .expect("task");
        state.tasks[index].finish(outcome, "done", "boom");
    }

    #[test]
    fn drafts_commit_plan_state_without_allocating() {
        let mut state = demo();
        let threads = state.threads.len();
        prepare(&mut state, "a", json!({})).unwrap();
        prepare(
            &mut state,
            "b",
            json!({"depends_on": ["a"], "expected_outputs": "diff"}),
        )
        .unwrap();
        prepare(&mut state, "c", json!({})).unwrap();
        assert_eq!(state.threads.len(), threads);
        assert!(state.tasks.is_empty());
        assert!(state.scheduler.reservations.is_empty());
        let view = state
            .mcp_call(1, "task_plan", &json!({"plan": "p"}))
            .unwrap();
        let nodes = view["plan"]["nodes"].as_array().unwrap();
        assert_eq!(nodes.len(), 3);
        assert_eq!(nodes[0]["status"], "ready");
        assert_eq!(nodes[1]["status"], "draft");
        assert_eq!(nodes[1]["expected_outputs"], "diff");
        // A second plan stays separate; task_plan without an id lists only the caller's scope.
        state
            .mcp_call(
                1,
                "task_prepare",
                &json!({"plan": "other", "node": "x", "prompt": "x"}),
            )
            .unwrap();
        let plans = state.mcp_call(1, "task_plan", &json!({})).unwrap();
        assert_eq!(plans["plans"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn updates_are_cas_revisions_on_drafts_only() {
        let mut state = demo();
        prepare(&mut state, "a", json!({"expected_outputs": "draft"})).unwrap();
        let stale = update(&mut state, "a", 9, json!({"prompt": "new brief"})).unwrap_err();
        assert!(stale.contains("changed since revision"), "{stale}");
        let missing = state
            .mcp_call(
                1,
                "task_update",
                &json!({"plan": "p", "node": "a", "prompt": "x"}),
            )
            .unwrap_err();
        assert!(missing.contains("expected_revision"), "{missing}");
        update(
            &mut state,
            "a",
            1,
            json!({"prompt": "revised brief", "expected_outputs": null}),
        )
        .unwrap();
        let node = &state.task_plans[0].nodes[0];
        assert_eq!(node.revision, 2);
        assert_eq!(node.brief, "revised brief");
        assert!(node.expected_outputs.is_none());
        start(
            &mut state,
            json!({"plan": "p", "node": "a", "client_request_id": "k"}),
        )
        .unwrap();
        let frozen = update(&mut state, "a", 2, json!({"prompt": "too late"})).unwrap_err();
        assert!(frozen.contains("already launched"), "{frozen}");
    }

    #[test]
    fn start_launches_selected_ready_nodes_in_dependency_order() {
        let mut state = demo();
        prepare(&mut state, "a", json!({})).unwrap();
        prepare(&mut state, "b", json!({"depends_on": ["a"]})).unwrap();
        prepare(&mut state, "c", json!({})).unwrap();
        let blocked = start(
            &mut state,
            json!({"plan": "p", "nodes": ["b"], "client_request_id": "k1"}),
        )
        .unwrap_err();
        assert!(blocked.contains("not ready"), "{blocked}");
        assert!(state.tasks.is_empty());
        state.scheduler.limits.account_pending = Some(1);
        let over_capacity = start(
            &mut state,
            json!({"plan": "p", "nodes": ["a", "c"], "client_request_id": "too-many"}),
        )
        .unwrap_err();
        assert!(over_capacity.contains("admission queue"), "{over_capacity}");
        assert!(state.tasks.is_empty());
        assert!(state.scheduler.reservations.is_empty());
        state.scheduler.limits.account_pending = None;
        // Independent nodes launch together under one keyed request.
        let out = start(
            &mut state,
            json!({"plan": "p", "nodes": ["a", "c"], "client_request_id": "k2"}),
        )
        .unwrap();
        assert_eq!(out["started"].as_array().unwrap().len(), 2);
        assert_eq!(state.tasks.len(), 2);
        assert_eq!(node_view(&state, "b")["status"], "draft");
        finish_task(&mut state, "a", TaskState::Completed);
        assert_eq!(node_view(&state, "b")["status"], "ready");
        start(
            &mut state,
            json!({"plan": "p", "node": "b", "client_request_id": "k3"}),
        )
        .unwrap();
        assert_eq!(state.tasks.len(), 3);
        let task = state
            .tasks
            .iter()
            .find(|task| task.plan_node.as_ref().is_some_and(|p| p.node == "b"))
            .expect("b task");
        let status = state
            .mcp_call(
                1,
                "task_status",
                &json!({"thread_id": task.child_thread_id}),
            )
            .unwrap();
        assert_eq!(status["plan"], json!({"id": "p", "node": "b"}));
    }

    #[test]
    fn a_failed_or_stopped_upstream_blocks_until_an_audited_override() {
        let mut state = demo();
        prepare(&mut state, "a", json!({})).unwrap();
        prepare(&mut state, "b", json!({"depends_on": ["a"]})).unwrap();
        start(
            &mut state,
            json!({"plan": "p", "node": "a", "client_request_id": "ka"}),
        )
        .unwrap();
        finish_task(&mut state, "a", TaskState::Failed);
        let dep = node_view(&state, "b")["dependencies"][0].clone();
        assert_eq!(dep["state"], "failed");
        assert!(dep["detail"].as_str().unwrap().contains("boom"), "{dep}");
        let blocked = start(
            &mut state,
            json!({"plan": "p", "node": "b", "client_request_id": "kb"}),
        )
        .unwrap_err();
        assert!(
            blocked.contains("not ready") && blocked.contains("boom"),
            "{blocked}"
        );
        // A released or cancelled upstream still does not silently unblock.
        let index = 0;
        state.tasks[index].state = TaskState::Cancelled;
        assert_eq!(
            node_view(&state, "b")["dependencies"][0]["state"],
            "cancelled"
        );
        assert_eq!(node_view(&state, "b")["status"], "blocked");
        start(
            &mut state,
            json!({"plan": "p", "node": "b", "client_request_id": "kb2", "override_reason": "upstream flake, output already reviewed"}),
        )
        .unwrap();
        let plan = &state.task_plans[0];
        assert_eq!(plan.overrides.len(), 1);
        let audit = &plan.overrides[0];
        assert_eq!(audit.node, "b");
        assert_eq!(audit.unsatisfied[0].node, "a");
        assert_eq!(audit.unsatisfied[0].state, "cancelled");
        assert!(audit.reason.contains("flake"));
        assert_eq!(
            node_view(&state, "b")["dependencies"][0]["state"],
            "overridden"
        );
        assert_eq!(
            state.task_plans[0].nodes[1]
                .launch
                .as_ref()
                .unwrap()
                .overrode,
            vec!["a".to_owned()]
        );
    }

    #[test]
    fn invalid_graphs_and_targets_fail_before_launch() {
        let mut state = demo();
        let missing = prepare(&mut state, "a", json!({"depends_on": ["nope"]})).unwrap_err();
        assert!(missing.contains("not in this plan"), "{missing}");
        let self_dep = prepare(&mut state, "a", json!({"depends_on": ["a"]})).unwrap_err();
        assert!(self_dep.contains("itself"), "{self_dep}");
        let bad_target =
            prepare(&mut state, "a", json!({"target": {"harness": "bogus"}})).unwrap_err();
        assert!(!bad_target.is_empty());
        let linked = prepare(&mut state, "a", json!({"target":{"machine":"Studio"}})).unwrap_err();
        assert!(linked.contains("local machine only"), "{linked}");
        assert!(
            state.task_plans.is_empty(),
            "refused draft left an empty plan"
        );
        prepare(&mut state, "a", json!({})).unwrap();
        // stacked_on must name a declared dependency.
        let bad_stack = prepare(
            &mut state,
            "b",
            json!({"depends_on": ["a"], "stacked_on": "c"}),
        )
        .unwrap_err();
        assert!(bad_stack.contains("without depending"), "{bad_stack}");
        let ghost_stack = prepare(
            &mut state,
            "b",
            json!({"depends_on": ["ghost"], "stacked_on": "ghost"}),
        )
        .unwrap_err();
        assert!(ghost_stack.contains("not in this plan"), "{ghost_stack}");
        let bad_stack = prepare(&mut state, "b", json!({"stacked_on": "a"})).unwrap_err();
        assert!(bad_stack.contains("without depending"), "{bad_stack}");
        prepare(&mut state, "b", json!({"depends_on": ["a"]})).unwrap();
        // A cycle introduced by an edit is refused before it can launch.
        let cycle = update(&mut state, "a", 1, json!({"depends_on": ["b"]})).unwrap_err();
        assert!(cycle.contains("cycle"), "{cycle}");
        // A stacked node cannot also pin its own base.
        let bad_base = prepare(
            &mut state,
            "d",
            json!({"depends_on": ["a"], "stacked_on": "a", "placement": {"mode": "isolated", "baseBranch": "main"}}),
        )
        .unwrap_err();
        assert!(bad_base.contains("base"), "{bad_base}");
        let unknown = start(
            &mut state,
            json!({"plan": "p", "node": "ghost", "client_request_id": "k"}),
        )
        .unwrap_err();
        assert!(unknown.contains("no node 'ghost'"), "{unknown}");
        prepare(
            &mut state,
            "c",
            json!({"placement":{"mode":"isolated","baseBranch":"missing"}}),
        )
        .unwrap();
        let before = state.next_id;
        let placement = start(
            &mut state,
            json!({"plan":"p","nodes":["a","c"],"client_request_id":"bad-placement"}),
        )
        .unwrap_err();
        assert!(placement.contains("Plan node 'c'"), "{placement}");
        assert_eq!(state.next_id, before);
        assert!(state.tasks.is_empty());
    }

    #[test]
    fn start_replays_identical_retries_and_rejects_reused_keys() {
        let mut state = demo();
        prepare(&mut state, "a", json!({})).unwrap();
        let args = json!({"plan": "p", "node": "a", "client_request_id": "k"});
        let first = start(&mut state, args.clone()).unwrap();
        assert_eq!(first["replayed"], json!(null));
        let tasks = state.tasks.len();
        let replay = start(&mut state, args).unwrap();
        assert_eq!(replay["replayed"], true);
        assert_eq!(
            replay["started"][0]["task_id"],
            first["started"][0]["task_id"]
        );
        assert_eq!(state.tasks.len(), tasks);
        let reused = start(
            &mut state,
            json!({"plan": "p", "nodes": ["a"], "client_request_id": "k"}),
        )
        .unwrap_err();
        assert_eq!(reused, LAUNCH_KEY_MISMATCH);
        let unkeyed = start(&mut state, json!({"plan": "p", "node": "a"})).unwrap_err();
        assert!(unkeyed.contains("client_request_id"), "{unkeyed}");
        // A node already launched under another key cannot launch again.
        let again = start(
            &mut state,
            json!({"plan": "p", "node": "a", "client_request_id": "k2"}),
        )
        .unwrap_err();
        assert!(again.contains("already launched"), "{again}");
    }

    #[test]
    fn plans_and_launch_receipts_survive_restart() {
        let mut state = demo();
        prepare(&mut state, "a", json!({})).unwrap();
        update(&mut state, "a", 1, json!({"title": "A revised"})).unwrap();
        prepare(&mut state, "b", json!({"depends_on": ["a"]})).unwrap();
        let start_args = json!({"plan": "p", "node": "a", "client_request_id": "k"});
        start(&mut state, start_args.clone()).unwrap();
        let snapshot = state.snapshot();
        let mut restored = demo();
        restored.restore_records(&snapshot, false).unwrap();
        let plan = &restored.task_plans[0];
        assert_eq!(plan.nodes.len(), 2);
        assert_eq!(plan.nodes[0].revision, 2);
        assert_eq!(plan.nodes[0].title, "A revised");
        assert!(plan.nodes[0].launch.is_some());
        assert_eq!(plan.nodes[1].depends_on, vec!["a".to_owned()]);
        let replay = restored.mcp_call(1, "task_start", &start_args).unwrap();
        assert_eq!(replay["replayed"], true);
        assert_eq!(restored.tasks.len(), state.tasks.len());
    }

    #[test]
    fn delegate_task_still_launches_immediately() {
        let mut state = demo();
        let out = state
            .mcp_call(1, "delegate_task", &json!({"prompt": "direct work"}))
            .unwrap();
        assert!(matches!(
            out["status"].as_str(),
            Some("queued" | "starting")
        ));
        assert_eq!(state.tasks.len(), 1);
        assert!(state.task_plans.is_empty());
    }

    /// A stacked node resolves the verified HEAD of the upstream worktree and
    /// passes exactly that commit to placement, never a branch default.
    #[test]
    fn stacked_nodes_base_on_the_verified_upstream_commit() {
        let root = std::env::temp_dir().join(format!("shina-plan-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let cfg = root.join("empty.gitconfig");
        std::fs::write(&cfg, "").unwrap();
        let repo = root.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .arg("-c")
                .arg("user.name=Test")
                .arg("-c")
                .arg("user.email=t@e.c")
                .args(args)
                .current_dir(&repo)
                .env("GIT_CONFIG_GLOBAL", &cfg)
                .env("GIT_CONFIG_SYSTEM", &cfg)
                .output()
                .expect("git");
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8_lossy(&out.stdout).into_owned()
        };
        git(&["init", "-b", "main", "--quiet"]);
        std::fs::write(repo.join("f"), "one\n").unwrap();
        git(&["add", "f"]);
        git(&["commit", "-m", "one", "--quiet"]);
        let first = git(&["rev-parse", "HEAD"]).trim().to_owned();
        std::fs::write(repo.join("f"), "two\n").unwrap();
        git(&["commit", "-am", "two", "--quiet"]);
        let head = git(&["rev-parse", "HEAD"]).trim().to_owned();
        assert_ne!(first, head);

        let mut state = demo();
        state.test_git_context = Some(crate::vcs::GitContext {
            git_config_global: Some(cfg.clone()),
            git_config_system: Some(cfg.clone()),
        });
        prepare(&mut state, "up", json!({})).unwrap();
        // Fabricate the upstream launch: a completed task on a worktree thread.
        let mut thread = Thread::new(2);
        thread.parent = Some(1);
        thread.delegated = true;
        thread.worktree_path = Some(repo.to_string_lossy().into_owned());
        thread.placement = Some(placement::ThreadPlacement {
            requested: placement::PlacementRequest::Default,
            mode: placement::PlacementMode::Isolated,
            phase: placement::PlacementPhase::Ready,
            executor: "local".into(),
            repository: None,
            base_branch: None,
            base_commit: None,
            upstream_thread_id: None,
            branch: Some("worker-a".into()),
            checkout: None,
            start_from_origin: false,
            primary_checkout: false,
            new_worktree: true,
            capability: None,
            error: None,
            binding: None,
        });
        let mut task = Task::new(&thread, "upstream", None);
        task.state = TaskState::Completed;
        let task_id = task.id.clone();
        state.threads.push(thread);
        state.tasks.push(task);
        state.task_plans[0].nodes[0].launch = Some(NodeLaunch {
            request_key: "k".into(),
            delegate_key: plan_delegate_key("p", "up"),
            task_id,
            thread_id: 2,
            launched_at: 0,
            overrode: Vec::new(),
            resolved_base: None,
        });
        prepare(
            &mut state,
            "down",
            json!({"depends_on": ["up"], "stacked_on": "up", "placement": {"mode": "isolated", "branch": "stacked-b"}}),
        )
        .unwrap();
        let ctx = state.test_git_context.clone().unwrap_or_default();
        let (plan, down) = {
            let plan = &state.task_plans[0];
            (plan.clone(), plan.nodes[1].clone())
        };
        let resolved = state.resolve_stacked_base(&plan, &down, &ctx).unwrap();
        assert_eq!(resolved.commit, head);
        assert_eq!(resolved.branch.as_deref(), Some("worker-a"));
        assert_eq!(resolved.node, "up");
        // An uncommitted change is reported but the base stays the commit.
        std::fs::write(repo.join("dirty"), "x").unwrap();
        let dirty = state.resolve_stacked_base(&plan, &down, &ctx).unwrap();
        assert!(dirty.uncommitted);
        assert_eq!(dirty.commit, head);
        let args = state.plan_delegate_args(&plan, &down, Some(&resolved));
        assert_eq!(args["placement"]["baseCommit"], head);
        assert_eq!(args["placement"]["branch"], "stacked-b");
        // An unfinished upstream can never supply a base.
        state.tasks[0].state = TaskState::Running;
        let unfinished = state.resolve_stacked_base(&plan, &down, &ctx).unwrap_err();
        assert!(unfinished.contains("not completed"), "{unfinished}");
        let _ = std::fs::remove_dir_all(&root);
    }
}
