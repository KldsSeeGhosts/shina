//! Provider-owned agent runs stay inside the originating thread's tools.

use crate::model::{Thread, Tool};
use serde_json::Value;
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::time::Instant;

pub use crate::model::ToolAnchor;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentState {
    Queued,
    Working,
    Done,
    Failed,
    Stopped,
}

impl AgentState {
    pub fn active(self) -> bool {
        matches!(self, Self::Queued | Self::Working)
    }

    fn for_tool(tool: &Tool) -> Self {
        if !tool.done {
            Self::Working
        } else if tool.failed {
            Self::Failed
        } else {
            Self::Done
        }
    }

    fn for_result(run: &Value, tool: &Tool) -> Self {
        match run["status"].as_str() {
            Some("Queued" | "Working")
                if tool.done
                    && tool.input["type"] != "collabAgentToolCall"
                    && tool.input["type"] != "subAgentActivity" =>
            {
                Self::for_tool(tool)
            }
            Some("Queued") => Self::Queued,
            Some("Working") => Self::Working,
            Some("Stopped") => Self::Stopped,
            Some("Done") => Self::Done,
            Some("Failed") => Self::Failed,
            _ => Self::for_tool(tool),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AgentsFilter {
    #[default]
    All,
    Active,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AgentIdentity<'a> {
    Provider(&'a str),
    Index(usize),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum AgentRunId {
    Provider(String),
    Index(usize),
}

pub struct AgentRun<'a> {
    pub tool_id: &'a str,
    pub tool_position: usize,
    pub anchor: ToolAnchor,
    pub index: usize,
    pub provider_id: Option<&'a str>,
    pub name: Cow<'a, str>,
    pub brief: &'a str,
    pub model: Option<&'a str>,
    pub state: AgentState,
    pub started: Option<Instant>,
    pub duration_ms: Option<u64>,
    pub activity: Option<&'a str>,
    pub output: &'a str,
    pub count: usize,
    key_by_index: bool,
}

impl<'a> AgentRun<'a> {
    pub fn key(&self) -> (&'a str, AgentIdentity<'a>) {
        (
            self.tool_id,
            self.provider_id
                .filter(|_| !self.key_by_index)
                .map_or(AgentIdentity::Index(self.index), AgentIdentity::Provider),
        )
    }

    pub fn owned_key(&self, server: &str, thread: usize) -> (String, usize, String, AgentRunId) {
        let (tool, identity) = self.key();
        (
            server.into(),
            thread,
            tool.into(),
            match identity {
                AgentIdentity::Provider(id) => AgentRunId::Provider(id.into()),
                AgentIdentity::Index(index) => AgentRunId::Index(index),
            },
        )
    }
}

#[derive(Default)]
pub(crate) struct AgentIndex {
    tools: BTreeMap<usize, (usize, usize)>,
    latest_nested: HashMap<String, usize>,
    active: usize,
    total: usize,
    tool_count: usize,
}

fn aggregate(tool: &Tool) -> usize {
    if tool.resolved_kind() != crate::work::ToolKind::Subagent {
        return 0;
    }
    [&tool.structured["details"], &tool.input, &tool.structured]
        .into_iter()
        .flat_map(|value| [value["agentCount"].as_u64(), value["count"].as_u64()])
        .flatten()
        .find(|count| *count > 0)
        .and_then(|count| usize::try_from(count).ok())
        .unwrap_or_default()
}

impl AgentIndex {
    pub fn observe(&mut self, index: usize, tool: &Tool) {
        self.tool_count = self.tool_count.max(index + 1);
        if let Some(parent) = &tool.parent_call {
            let latest = self.latest_nested.entry(parent.clone()).or_insert(index);
            *latest = (*latest).max(index);
        }
        let (active, total) = if tool.subagents.is_empty() {
            let count = aggregate(tool);
            if count == 0 {
                if let Some((active, total)) = self.tools.remove(&index) {
                    self.active = self.active.saturating_sub(active);
                    self.total = self.total.saturating_sub(total);
                }
                return;
            }
            (if tool.done { 0 } else { count }, count)
        } else {
            (
                tool.subagents
                    .iter()
                    .filter(|run| AgentState::for_result(run, tool).active())
                    .count(),
                tool.subagents.len(),
            )
        };
        let (old_active, old_total) = self
            .tools
            .insert(index, (active, total))
            .unwrap_or_default();
        self.active = self
            .active
            .saturating_sub(old_active)
            .saturating_add(active);
        self.total = self.total.saturating_sub(old_total).saturating_add(total);
    }

    pub fn rebuild(tools: &[Tool]) -> Self {
        let mut summary = Self::default();
        for (index, tool) in tools.iter().enumerate() {
            summary.observe(index, tool);
        }
        summary
    }

    fn fallback(&self, tools: &[Tool]) -> Option<Self> {
        // Directly constructed threads can bypass ingestion. Normal provider
        // changes and projection replacements maintain the index at the source.
        (self.tool_count != tools.len()).then(|| Self::rebuild(tools))
    }
}

pub fn active_count(thread: &Thread) -> usize {
    thread.projected_agent_counts.map_or_else(
        || {
            thread
                .agent_index
                .fallback(&thread.tools)
                .as_ref()
                .unwrap_or(&thread.agent_index)
                .active
        },
        |(active, _)| active,
    )
}

pub fn has_agents(thread: &Thread) -> bool {
    total_count(thread) != 0
}

pub fn total_count(thread: &Thread) -> usize {
    thread.projected_agent_counts.map_or_else(
        || {
            thread
                .agent_index
                .fallback(&thread.tools)
                .as_ref()
                .unwrap_or(&thread.agent_index)
                .total
        },
        |(_, total)| total,
    )
}

pub fn runs(thread: &Thread) -> Vec<AgentRun<'_>> {
    let mut runs = Vec::new();
    let fallback = thread.agent_index.fallback(&thread.tools);
    let index_data = fallback.as_ref().unwrap_or(&thread.agent_index);
    for &tool in index_data.tools.keys().rev() {
        let len = thread.tools[tool].subagents.len().max(1);
        runs.extend((0..len).filter_map(|position| run_at(thread, index_data, tool, position)));
    }
    runs
}

pub fn run(thread: &Thread, tool: usize, position: usize) -> Option<AgentRun<'_>> {
    let fallback = thread.agent_index.fallback(&thread.tools);
    run_at(
        thread,
        fallback.as_ref().unwrap_or(&thread.agent_index),
        tool,
        position,
    )
}

fn run_at<'a>(
    thread: &'a Thread,
    data: &AgentIndex,
    tool_position: usize,
    index: usize,
) -> Option<AgentRun<'a>> {
    let tool = thread.tools.get(tool_position)?;
    let activity = data
        .latest_nested
        .get(&tool.id)
        .and_then(|&i| thread.tools.get(i))
        .map(|nested| {
            if nested.detail.is_empty() {
                nested.name.as_str()
            } else {
                nested.detail.as_str()
            }
        });
    if tool.subagents.is_empty() {
        let count = aggregate(tool);
        return (count > 0).then(|| AgentRun {
            tool_id: &tool.id,
            tool_position,
            anchor: tool.anchor,
            index: 0,
            provider_id: None,
            name: format!("{count} agents").into(),
            brief: "",
            model: None,
            state: AgentState::for_tool(tool),
            started: tool.started,
            duration_ms: tool.duration_ms,
            activity,
            output: &tool.output,
            count,
            key_by_index: true,
        });
    }
    let agent = tool.subagents.get(index)?;
    let state = AgentState::for_result(agent, tool);
    Some(AgentRun {
        tool_id: &tool.id,
        tool_position,
        anchor: tool.anchor,
        index,
        provider_id: agent["providerId"].as_str(),
        name: agent["name"]
            .as_str()
            .filter(|name| !name.is_empty())
            .map_or_else(|| format!("Agent {}", index + 1).into(), Cow::Borrowed),
        brief: agent["task"].as_str().unwrap_or(""),
        model: agent["model"].as_str(),
        state,
        started: tool.started,
        duration_ms: agent["durationMs"].as_u64().or(tool.duration_ms),
        activity: state
            .active()
            .then(|| agent["activity"].as_str().or(activity))
            .flatten(),
        output: agent["output"].as_str().unwrap_or(""),
        count: 1,
        key_by_index: agent["keyByIndex"] == true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::State;
    use serde_json::json;

    #[test]
    fn pi_input_and_partial_results_keep_stable_runs_without_creating_threads() {
        let mut state = State::headless(crate::store::directory().unwrap());
        let thread_count = state.threads.len();
        state.current_mut().apply(json!({"type":"tool_execution_start","toolCallId":"call","toolName":"subagent",
            "args":{"tasks":[{"agent":"scout","task":"Find the source"},{"agent":"test","task":"Check it"}]}}));
        let runs_before = runs(state.current());
        assert_eq!(runs_before.len(), 2);
        assert_eq!(runs_before[0].state, AgentState::Queued);
        assert_eq!(active_count(state.current()), 2);

        // A partial result for the second task must not renumber the first.
        state.current_mut().apply(json!({"type":"tool_execution_update","toolCallId":"call","partialResult":{"details":{"results":[
            {"agent":"test","task":"Check it","finished":false,"activity":"Running checks","messages":[{"role":"assistant","content":[{"type":"text","text":"Halfway"}]}]}
        ]}}}));
        let working = runs(state.current());
        assert_eq!(working[0].key(), ("call", AgentIdentity::Index(0)));
        assert_eq!(working[1].key(), ("call", AgentIdentity::Index(1)));
        assert_eq!(working[1].activity, Some("Running checks"));
        assert_eq!(working[1].output, "Halfway");
        state.current_mut().apply(json!({"type":"tool_execution_end","toolCallId":"call","result":{"details":{"results":[
            {"agent":"scout","task":"Find the source","finished":true},
            {"agent":"test","task":"Check it","finished":true,"exitCode":1,"errorMessage":"Check failed"}
        ]}}}));
        assert_eq!(active_count(state.current()), 0);
        assert_eq!(runs(state.current())[1].state, AgentState::Failed);
        assert_eq!(state.threads.len(), thread_count);
        state.reveal_tool("call".into());
        assert!(state.current().tools[0].expanded);
        assert_eq!(
            state.flash_tool.as_ref().map(|(id, _)| id.as_str()),
            Some("call")
        );
    }

    #[test]
    fn count_only_provider_yields_one_aggregate_row() {
        let mut thread = Thread::new(1);
        thread.apply(json!({"type":"tool_execution_start","toolCallId":"total","toolName":"subagent","args":{"agentCount":3}}));
        let rows = runs(&thread);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "3 agents");
        assert_eq!(rows[0].count, 3);
        assert_eq!(active_count(&thread), 3);
        thread.apply(
            json!({"type":"tool_execution_end","toolCallId":"total","durationMs":90,"result":{}}),
        );
        assert_eq!(runs(&thread)[0].duration_ms, Some(90));
        assert_eq!(active_count(&thread), 0);
        assert!(has_agents(&thread));
    }
}
