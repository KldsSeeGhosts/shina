//! Desktop state for a Captain home's task plan panel (issue #34). The panel
//! never builds its own plan model: it renders the value the MCP `task_plan`
//! tool returns for the same caller, and every edit or launch is a
//! `task.plan.*` command that the server routes to the same reducer as
//! `task_prepare`, `task_update` and `task_start`, as the Captain home.
//! Without a client this desktop is the server, so the command executes
//! locally through `execute_command` and the view is read live.
use super::*;
use crate::protocol::Command;
use rune::TextEdit;

/// A draft being created or revised in the panel.
#[derive(Clone, Debug, Default)]
pub struct PlanEditor {
    /// The plan the draft belongs to. `None` with `node: None` creates a plan.
    pub plan: Option<String>,
    /// The node being revised, or `None` for a new draft.
    pub node: Option<String>,
    /// The revision the edit started from; `task_update` checks it.
    pub revision: u64,
    pub key: TextEdit,
    pub title: TextEdit,
    pub brief: TextEdit,
    pub outputs: TextEdit,
    pub depends_on: Vec<String>,
    pub stacked_on: Option<String>,
    /// The server refused the save because the draft changed since `revision`.
    pub conflict: bool,
}

/// The selected nodes the user is about to start, held while an override is
/// confirmed. `key` is reused verbatim when the same start is retried.
#[derive(Clone, Debug, Default)]
pub struct PlanStart {
    pub plan: String,
    pub nodes: Vec<String>,
    pub key: String,
    /// The start needs an override and the user has not confirmed it yet.
    pub confirming: bool,
}

#[derive(Default)]
pub struct PlanPanel {
    /// The machine and Captain home thread the panel is open for.
    pub open: Option<(String, usize)>,
    /// The last `task_plan` value the owning server returned, and for whom.
    pub view: Option<(String, usize, Value)>,
    /// The plan shown when the scope has several.
    pub plan: Option<String>,
    pub selected: std::collections::BTreeSet<String>,
    pub editor: Option<PlanEditor>,
    pub start: Option<PlanStart>,
    pub override_reason: TextEdit,
    /// The command in flight, so a second press waits for its receipt.
    pub pending: Option<uuid::Uuid>,
    pub reading: Option<uuid::Uuid>,
    /// Task identities and states the current view was read against.
    pub signature: Option<u64>,
    pub error: Option<String>,
    pub notice: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_connected_panel_refreshes_mcp_draft_edits_without_a_task_change() {
        let mut state = State::new(std::path::PathBuf::new(), false);
        state.demo = true;
        state.use_pi_for_test();
        state.captains = Captain::presets();
        state.open_captain(0);
        let home = state.current().id;
        state
            .mcp_call(
                home,
                "task_prepare",
                &json!({"plan":"p","node":"a","prompt":"Read code."}),
            )
            .unwrap();
        let (client, mut outgoing) = crate::client::Client::test_connected(state.data_dir.clone());
        client.info.lock().unwrap()["capabilities"] = json!([crate::protocol::TASK_PLANS]);
        state.client = Some(client);
        state.toggle_plan_panel();
        let request = outgoing.try_recv().unwrap();
        assert_eq!(request.method, "task.plan");
        assert_eq!(request.params["threadId"], home);
        let initial = state.task_plans_view(state.active);
        assert!(state.task_plan_receipt(
            "local",
            &request,
            &json!({"result":{"status":"accepted","result":initial}}),
        ));
        assert!(!state.poll_plan_panel());

        state.task_plans[0].nodes[0].revision += 1;
        assert!(state.tasks_for(home).is_empty());
        assert!(state.poll_plan_panel());
        let refresh = outgoing.try_recv().unwrap();
        assert_eq!(refresh.method, "task.plan");
        assert_eq!(refresh.params, request.params);
        assert_ne!(refresh.command_id, request.command_id);
    }
}

fn editor_text(text: &str) -> TextEdit {
    let mut edit = TextEdit::default();
    edit.replace_text(text);
    edit
}

impl State {
    /// The open thread is a primary Captain home that may own plans.
    pub(crate) fn plan_home(&self) -> Option<usize> {
        let thread = self.current();
        let captain = thread.captain?;
        (self.is_captain_home(thread)
            && thread.peer_origin.is_none()
            && self.captain_home(captain) == Some(thread.id))
        .then_some(thread.id)
    }

    /// `task-plans-v1` on the selected machine. Without a client this
    /// desktop is the server and always has it.
    pub(crate) fn plan_capability(&self) -> Result<(), String> {
        let Some(client) = &self.client else {
            return Ok(());
        };
        if !self.machine_connected(&self.machines.selected) {
            return Err(format!(
                "{} is unreachable.",
                self.machine_label(&self.machines.selected)
            ));
        }
        if client.supports_capability(&self.machines.selected, crate::protocol::TASK_PLANS) {
            Ok(())
        } else {
            Err(crate::protocol::TASK_PLANS_UPDATE_REQUIRED.into())
        }
    }

    pub(crate) fn plan_panel_open(&self) -> bool {
        self.plan_home().is_some_and(|home| {
            self.plan_panel.open.as_ref() == Some(&(self.machines.selected.clone(), home))
        })
    }

    pub fn toggle_plan_panel(&mut self) {
        let Some(home) = self.plan_home() else {
            return;
        };
        if self.plan_panel_open() {
            self.plan_panel = PlanPanel::default();
            return;
        }
        self.plan_panel = PlanPanel {
            open: Some((self.machines.selected.clone(), home)),
            ..PlanPanel::default()
        };
        if let Err(error) = self.plan_capability() {
            self.plan_panel.error = Some(error);
            return;
        }
        self.request_plan_view();
    }

    /// Every plan in the Captain home's scope, exactly as `task_plan` returns it.
    pub(crate) fn plan_view(&self) -> Option<Value> {
        let (server, home) = self.plan_panel.open.clone()?;
        if self.client.is_none() {
            let index = self.threads.iter().position(|t| t.id == home)?;
            return Some(self.task_plans_view(index));
        }
        self.plan_panel
            .view
            .as_ref()
            .filter(|(at, id, _)| *at == server && *id == home)
            .map(|(_, _, value)| value.clone())
    }

    /// The plan the panel shows: the chosen one, else the latest edited.
    pub(crate) fn shown_plan(&self) -> Option<Value> {
        let view = self.plan_view()?;
        let plans = view["plans"].as_array()?;
        if let Some(id) = &self.plan_panel.plan
            && let Some(plan) = plans.iter().find(|plan| plan["id"] == *id)
        {
            return Some(plan.clone());
        }
        plans
            .iter()
            .max_by_key(|plan| (plan["updated_at"].as_u64(), plan["id"].as_str()))
            .cloned()
    }

    fn task_signature(&self, home: usize) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for task in self.tasks_for(home) {
            task.id.hash(&mut hasher);
            task.state.id().hash(&mut hasher);
        }
        if let Some(index) = self.threads.iter().position(|thread| thread.id == home) {
            let scope = self.delegate_scope_device(index);
            for plan in self.task_plans.iter().filter(|plan| plan.scope == scope) {
                plan.id.hash(&mut hasher);
                for node in &plan.nodes {
                    node.key.hash(&mut hasher);
                    node.revision.hash(&mut hasher);
                    node.launch
                        .as_ref()
                        .map(|launch| &launch.task_id)
                        .hash(&mut hasher);
                }
            }
        }
        hasher.finish()
    }

    fn request_plan_view(&mut self) {
        let Some((server, home)) = self.plan_panel.open.clone() else {
            return;
        };
        if server != self.machines.selected || self.plan_home() != Some(home) {
            return;
        }
        self.plan_panel.signature = Some(self.task_signature(home));
        if self.client.is_none() {
            return;
        }
        match self.remote("task.plan", json!({"threadId": home, "args": {}})) {
            Some(Ok(id)) => self.plan_panel.reading = Some(id),
            Some(Err(error)) => self.plan_panel.error = Some(error),
            None => {}
        }
    }

    /// Reread when this home's tasks or draft revisions change, including
    /// edits from MCP or from another desktop.
    pub(crate) fn poll_plan_panel(&mut self) -> bool {
        let Some((server, home)) = self.plan_panel.open.clone() else {
            return false;
        };
        if server != self.machines.selected
            || self.plan_home() != Some(home)
            || self.plan_panel.reading.is_some()
        {
            return false;
        }
        if self.plan_panel.signature == Some(self.task_signature(home)) {
            return false;
        }
        self.request_plan_view();
        true
    }

    /// Send one `task.plan.*` command as the open Captain home.
    fn plan_command(&mut self, method: &str, args: Value) {
        let Some((server, home)) = self.plan_panel.open.clone() else {
            return;
        };
        if server != self.machines.selected || self.plan_home() != Some(home) {
            return;
        }
        if self.plan_panel.pending.is_some() {
            return;
        }
        if let Err(error) = self.plan_capability() {
            self.plan_panel.error = Some(error);
            return;
        }
        self.plan_panel.error = None;
        self.plan_panel.notice = None;
        let params = json!({"threadId": home, "args": args});
        if self.client.is_none() {
            let command = Command::new(method, params);
            let result = self.execute_command(&command);
            let value = match result {
                Ok(result) => json!({"result": {"status": "accepted", "result": result}}),
                Err(error) => json!({"result": {"status": "rejected", "result": {"error": error}}}),
            };
            self.plan_panel.pending = Some(command.command_id);
            self.task_plan_receipt(&server, &command, &value);
            return;
        }
        match self.remote(method, params) {
            Some(Ok(id)) => self.plan_panel.pending = Some(id),
            Some(Err(error)) => self.plan_panel.error = Some(error),
            None => {}
        }
    }

    /// Apply a `task.plan.*` receipt from the panel's machine.
    pub(crate) fn task_plan_receipt(
        &mut self,
        server: &str,
        command: &Command,
        value: &Value,
    ) -> bool {
        if !command.method.starts_with("task.plan") {
            return false;
        }
        let ours = self.plan_panel_open()
            && self.plan_panel.open.as_ref().is_some_and(|(at, home)| {
                at == server && command.params["threadId"].as_u64() == Some(*home as u64)
            });
        let panel = &mut self.plan_panel;
        let accepted = value["result"]["status"] == "accepted";
        let result = &value["result"]["result"];
        let error = value["error"]["message"]
            .as_str()
            .or(result["error"].as_str())
            .unwrap_or("The server refused the plan change.")
            .to_owned();
        if command.method == "task.plan" {
            if panel.reading == Some(command.command_id) {
                panel.reading = None;
                if !ours {
                    return true;
                }
                if accepted {
                    let home = command.params["threadId"].as_u64().unwrap_or_default() as usize;
                    panel.view = Some((server.to_owned(), home, result.clone()));
                } else {
                    panel.error = Some(error);
                }
            }
            return true;
        }
        if panel.pending != Some(command.command_id) {
            return true;
        }
        panel.pending = None;
        if !ours {
            return true;
        }
        if !accepted {
            if command.method == "task.plan.update" && error.contains("changed since revision") {
                if let Some(editor) = &mut panel.editor {
                    editor.conflict = true;
                }
            }
            // A refused start recorded no launch receipt, so the next attempt,
            // perhaps with a different reason, takes a fresh request key.
            if command.method == "task.plan.start"
                && let Some(start) = &mut panel.start
            {
                start.key = format!("desktop-start-{}", uuid::Uuid::new_v4());
            }
            panel.error = Some(error);
            self.request_plan_view();
            return true;
        }
        if let Some(id) = result["plan_id"].as_str() {
            panel.plan = Some(id.to_owned());
        }
        match command.method.as_str() {
            "task.plan.prepare" | "task.plan.update" => {
                panel.editor = None;
                panel.notice = Some(format!(
                    "Saved {} at revision {}.",
                    result["node"].as_str().unwrap_or("the draft"),
                    result["revision"]
                ));
            }
            _ => {
                let started = result["started"].as_array().map_or(0, Vec::len);
                panel.start = None;
                panel.selected.clear();
                panel.override_reason = TextEdit::default();
                panel.notice = Some(if started == 1 {
                    "Started 1 node.".into()
                } else {
                    format!("Started {started} nodes.")
                });
            }
        }
        self.request_plan_view();
        true
    }

    pub fn plan_select(&mut self, plan: &str) {
        self.plan_panel.plan = Some(plan.to_owned());
        self.plan_panel.selected.clear();
        self.plan_panel.start = None;
        self.plan_panel.editor = None;
    }

    pub fn plan_toggle_node(&mut self, node: &str) {
        let selected = &mut self.plan_panel.selected;
        if !selected.remove(node) {
            selected.insert(node.to_owned());
        }
        self.plan_panel.start = None;
    }

    /// Open the editor for a new draft in the shown plan, or a new plan.
    pub fn plan_new_draft(&mut self, new_plan: bool) {
        let plan = if new_plan {
            None
        } else {
            self.shown_plan()
                .and_then(|plan| plan["id"].as_str().map(str::to_owned))
        };
        let count = self
            .shown_plan()
            .filter(|_| plan.is_some())
            .and_then(|plan| plan["nodes"].as_array().map(Vec::len))
            .unwrap_or(0);
        let mut editor = PlanEditor {
            plan,
            key: editor_text(&format!("node-{}", count + 1)),
            ..PlanEditor::default()
        };
        editor.title.focused = true;
        self.plan_panel.editor = Some(editor);
        self.plan_panel.start = None;
        self.plan_panel.error = None;
    }

    /// Open the editor on a draft's latest revision.
    pub fn plan_edit_node(&mut self, node: &str) {
        let Some(plan) = self.shown_plan() else {
            return;
        };
        let Some(value) = plan["nodes"]
            .as_array()
            .and_then(|nodes| nodes.iter().find(|n| n["key"] == node))
        else {
            return;
        };
        let text = |field: &str| value[field].as_str().unwrap_or_default().to_owned();
        self.plan_panel.editor = Some(PlanEditor {
            plan: plan["id"].as_str().map(str::to_owned),
            node: Some(node.to_owned()),
            revision: value["revision"].as_u64().unwrap_or(0),
            key: editor_text(node),
            title: editor_text(&text("title")),
            brief: editor_text(&text("brief")),
            outputs: editor_text(&text("expected_outputs")),
            depends_on: value["depends_on"]
                .as_array()
                .map(|deps| {
                    deps.iter()
                        .filter_map(|dep| dep.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default(),
            stacked_on: value["stacked_on"].as_str().map(str::to_owned),
            conflict: false,
        });
        self.plan_panel.start = None;
        self.plan_panel.error = None;
    }

    /// After a revision conflict, keep the typed fields but take the
    /// server's latest revision, so the next save replaces it knowingly.
    pub fn plan_rebase_edit(&mut self) {
        let Some(editor) = &self.plan_panel.editor else {
            return;
        };
        let Some(node) = editor.node.clone() else {
            return;
        };
        let latest = self.shown_plan().and_then(|plan| {
            plan["nodes"]
                .as_array()?
                .iter()
                .find(|n| n["key"] == node.as_str())?["revision"]
                .as_u64()
        });
        if let (Some(editor), Some(latest)) = (&mut self.plan_panel.editor, latest) {
            editor.revision = latest;
            editor.conflict = false;
            self.plan_panel.error = None;
        }
    }

    pub fn plan_toggle_dependency(&mut self, dep: &str) {
        let Some(editor) = &mut self.plan_panel.editor else {
            return;
        };
        if let Some(at) = editor.depends_on.iter().position(|d| d == dep) {
            editor.depends_on.remove(at);
            if editor.stacked_on.as_deref() == Some(dep) {
                editor.stacked_on = None;
            }
        } else {
            editor.depends_on.push(dep.to_owned());
        }
    }

    pub fn plan_toggle_stack(&mut self, dep: &str) {
        let Some(editor) = &mut self.plan_panel.editor else {
            return;
        };
        if editor.stacked_on.as_deref() == Some(dep) {
            editor.stacked_on = None;
        } else if editor.depends_on.iter().any(|d| d == dep) {
            editor.stacked_on = Some(dep.to_owned());
        }
    }

    pub fn plan_save_draft(&mut self) {
        let Some(editor) = self.plan_panel.editor.clone() else {
            return;
        };
        let mut args = json!({
            "title": editor.title.text.trim(),
            "prompt": editor.brief.text,
            "expected_outputs": if editor.outputs.text.trim().is_empty() {
                Value::Null
            } else {
                json!(editor.outputs.text)
            },
            "depends_on": editor.depends_on,
            "stacked_on": editor.stacked_on,
        });
        match &editor.node {
            Some(node) => {
                args["plan"] = json!(editor.plan);
                args["node"] = json!(node);
                args["expected_revision"] = json!(editor.revision);
                self.plan_command("task.plan.update", args);
            }
            None => {
                if let Some(plan) = &editor.plan {
                    args["plan"] = json!(plan);
                }
                args["node"] = json!(editor.key.text.trim());
                self.plan_command("task.plan.prepare", args);
            }
        }
    }

    /// Selected nodes that are not ready, with the reason each one waits.
    pub(crate) fn plan_unready(&self) -> Vec<(String, String)> {
        let Some(plan) = self.shown_plan() else {
            return Vec::new();
        };
        plan["nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|node| {
                node["key"]
                    .as_str()
                    .is_some_and(|key| self.plan_panel.selected.contains(key))
                    && node["status"] != "ready"
                    && node["status"] != "launched"
            })
            .map(|node| {
                let reasons = node["dependencies"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|dep| dep["satisfied"] != true)
                    .filter_map(|dep| dep["detail"].as_str())
                    .collect::<Vec<_>>()
                    .join("; ");
                (node["key"].as_str().unwrap_or_default().to_owned(), reasons)
            })
            .collect()
    }

    /// Start the selected nodes. Any node that is not ready first asks for an
    /// explicit override and its reason, which the plan records.
    pub fn plan_start_selected(&mut self) {
        let Some(plan) = self
            .shown_plan()
            .and_then(|plan| plan["id"].as_str().map(str::to_owned))
        else {
            return;
        };
        let launchable: Vec<String> = self
            .shown_plan()
            .and_then(|plan| plan["nodes"].as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter(|node| node["status"] != "launched")
            .filter_map(|node| node["key"].as_str().map(str::to_owned))
            .filter(|key| self.plan_panel.selected.contains(key))
            .collect();
        if launchable.is_empty() {
            return;
        }
        let needs_override = !self.plan_unready().is_empty();
        let reason = self.plan_panel.override_reason.text.trim().to_owned();
        let same = self
            .plan_panel
            .start
            .as_ref()
            .is_some_and(|start| start.plan == plan && start.nodes == launchable);
        if !same {
            self.plan_panel.start = Some(PlanStart {
                plan: plan.clone(),
                nodes: launchable.clone(),
                key: format!("desktop-start-{}", uuid::Uuid::new_v4()),
                confirming: needs_override,
            });
            if needs_override {
                self.plan_panel.override_reason = TextEdit::default();
                self.plan_panel.override_reason.focused = true;
                return;
            }
        }
        if needs_override && reason.is_empty() {
            if let Some(start) = &mut self.plan_panel.start {
                start.confirming = true;
            }
            return;
        }
        let key = self
            .plan_panel
            .start
            .as_ref()
            .map(|start| start.key.clone());
        let mut args = json!({
            "plan": plan,
            "nodes": launchable,
            "client_request_id": key,
        });
        if needs_override {
            args["override_reason"] = json!(reason);
        }
        self.plan_command("task.plan.start", args);
    }

    pub fn plan_cancel_start(&mut self) {
        self.plan_panel.start = None;
        self.plan_panel.override_reason = TextEdit::default();
    }
}

#[cfg(test)]
impl State {
    /// Settle a launched task, as its worker finishing or failing would.
    pub(crate) fn finish_task_for_test(&mut self, task_id: &str, outcome: TaskState, error: &str) {
        let task = self
            .tasks
            .iter_mut()
            .find(|task| task.id == task_id)
            .expect("task");
        task.finish(outcome, "", error);
    }

    pub(crate) fn lift_coordinator_limit_for_test(&mut self) {
        self.scheduler.limits.coordinator_unfinished = None;
    }
}
