use super::*;

impl State {
    pub fn set_timeline_far_from_bottom(&mut self, thread_id: usize, far: bool) {
        let key = (self.machines.selected.clone(), thread_id);
        if far {
            self.timeline_far_from_bottom.insert(key, true);
        } else {
            self.timeline_far_from_bottom.remove(&key);
        }
    }
    pub fn timeline_is_far_from_bottom(&self, thread_id: usize) -> bool {
        self.timeline_far_from_bottom
            .get(&(self.machines.selected.clone(), thread_id))
            .copied()
            .unwrap_or(false)
    }
    pub fn set_timeline_at_bottom(&mut self, thread_id: usize, at: bool) {
        self.timeline_at_bottom
            .insert((self.machines.selected.clone(), thread_id), at);
    }
    pub fn thread_find_active(&self) -> bool {
        self.thread_find.open
            && self.thread_find.server == self.machines.selected
            && self.thread_find.thread_id == self.current().id
    }
    pub fn open_thread_find(&mut self) {
        let server = self.machines.selected.clone();
        let thread_id = self.current().id;
        let same = self.thread_find.open
            && self.thread_find.server == server
            && self.thread_find.thread_id == thread_id;
        if !same {
            self.thread_find = ThreadFindState {
                open: true,
                server,
                thread_id,
                ..ThreadFindState::default()
            };
        }
        self.thread_find.query.focused = true;
        self.thread_find.query.anchor = 0;
        self.thread_find.query.focus = self.thread_find.query.text.len();
        if !self.thread_find.query.text.trim().is_empty() {
            self.run_thread_find(None, 0);
        }
    }
    pub fn open_thread_find_on_query(
        &mut self,
        query: String,
        start: Option<crate::thread_find::ThreadFindStart>,
    ) {
        let server = self.machines.selected.clone();
        let thread_id = self.current().id;
        self.thread_find = ThreadFindState {
            open: true,
            server,
            thread_id,
            ..ThreadFindState::default()
        };
        let query: String = query
            .chars()
            .take(crate::thread_find::THREAD_FIND_QUERY_MAX_LENGTH)
            .collect();
        self.thread_find.query.replace_text(&query);
        self.thread_find.query.focused = true;
        self.thread_find.query.anchor = 0;
        self.thread_find.query.focus = self.thread_find.query.text.len();
        self.thread_find.last_typed = self.thread_find.query.text.clone();
        if !query.trim().is_empty() {
            self.run_thread_find(start.as_ref(), 0);
        }
    }
    pub fn close_thread_find(&mut self) {
        self.thread_find = ThreadFindState::default();
    }
    #[cfg(test)]
    pub fn set_thread_find_query(&mut self, query: String) {
        let query: String = query
            .chars()
            .take(crate::thread_find::THREAD_FIND_QUERY_MAX_LENGTH)
            .collect();
        self.thread_find.query.replace_text(&query);
        self.thread_find.last_typed = query;
        self.thread_find.debounce_until = None;
        self.run_thread_find(None, 0);
    }
    pub fn retry_thread_find(&mut self) {
        if !self.thread_find.open {
            return;
        }
        self.run_thread_find(None, 0);
    }
    pub fn step_thread_find(&mut self, delta: i64) {
        if !self.thread_find.open
            || self.thread_find.total_matches == 0
            || self.thread_find.loading
            || self.thread_find.unsupported
        {
            return;
        }
        let total = self.thread_find.total_matches;
        let next_index =
            ((self.thread_find.active_index as i64 + delta).rem_euclid(total as i64)) as usize;
        if let Some(entry) = self.thread_find.navigation.iter().find(|entry| {
            next_index >= entry.start_index && next_index < entry.start_index + entry.count
        }) {
            let matched = crate::thread_find::ThreadFindMatch {
                entry_id: entry.entry_id.clone(),
                occurrence: next_index - entry.start_index,
            };
            self.thread_find.active_index = next_index;
            self.thread_find.active_match = Some(matched.clone());
            self.thread_find.pending_reveal = Some(matched);
            return;
        }
        let start =
            self.thread_find
                .active_match
                .as_ref()
                .map(|m| crate::thread_find::ThreadFindStart {
                    entry_id: m.entry_id.clone(),
                    occurrence: m.occurrence,
                });
        self.run_thread_find(start.as_ref(), delta);
    }
    pub fn sync_thread_find_input(&mut self) -> bool {
        if !self.thread_find.open {
            return false;
        }
        let clipped: String = self
            .thread_find
            .query
            .text
            .chars()
            .take(crate::thread_find::THREAD_FIND_QUERY_MAX_LENGTH)
            .collect();
        if clipped != self.thread_find.query.text {
            self.thread_find.query.replace_text(&clipped);
        }
        if self.thread_find.query.text == self.thread_find.last_typed {
            return false;
        }
        self.thread_find.last_typed = self.thread_find.query.text.clone();
        if self.client.is_some() {
            self.thread_find.debounce_until = Some(Instant::now() + Duration::from_millis(100));
        } else {
            self.thread_find.debounce_until = None;
            self.run_thread_find(None, 0);
        }
        true
    }
    pub fn poll_thread_find(&mut self, tx: &mut Tx) -> bool {
        let mut changed = self.sync_thread_find_input();
        if let Some(until) = self.thread_find.debounce_until {
            let now = Instant::now();
            if now >= until {
                self.thread_find.debounce_until = None;
                self.run_thread_find(None, 0);
                changed = true;
            } else {
                tx.wake_after(until.saturating_duration_since(now));
            }
        }
        changed |= self.consume_pending_find_reveal(tx);
        changed
    }
    pub(super) fn run_thread_find(
        &mut self,
        start: Option<&crate::thread_find::ThreadFindStart>,
        offset: i64,
    ) {
        if !self.thread_find.open {
            return;
        }
        let query = self.thread_find.query.text.trim().to_owned();
        if query.is_empty() {
            self.thread_find.loading = false;
            self.thread_find.error = None;
            self.thread_find.unsupported = false;
            self.thread_find.total_matches = 0;
            self.thread_find.active_index = 0;
            self.thread_find.active_match = None;
            self.thread_find.navigation.clear();
            self.thread_find.pending_reveal = None;
            return;
        }
        self.thread_find.request_id += 1;
        let req_id = self.thread_find.request_id;
        let thread_id = self.thread_find.thread_id;
        if let Some(client) = &self.client {
            let target = if self.thread_find.server == "local" {
                Some(client)
            } else {
                client.linked.get(&self.thread_find.server)
            };
            let Some(target) = target else {
                self.thread_find.loading = false;
                self.thread_find.error = Some("Machine unavailable.".into());
                return;
            };
            if !target.has_capability(crate::protocol::THREAD_FIND) {
                self.thread_find.loading = false;
                self.thread_find.unsupported = true;
                self.thread_find.error = Some(crate::protocol::THREAD_FIND_UPDATE_REQUIRED.into());
                return;
            }
            self.thread_find.unsupported = false;
            self.thread_find.error = None;
            self.thread_find.loading = true;
            let mut params = json!({"threadId": thread_id, "query": query});
            if let Some(start) = start {
                params["start"] = json!(start);
            }
            if offset != 0 {
                params["offset"] = json!(offset);
            }
            target.call_rpc(format!("find:{req_id}"), "thread.find", params);
            return;
        }
        let t = self.current();
        let messages: Vec<_> = t.messages.iter().map(|message|
            json!({"id":message.id,"role":message.role,"text":message.display_text()})).collect();
        let streams = crate::history::Streams::new(&json!({"seq":0,
            "state":{"threads":[{"id":t.id,"messages":messages,"tools":t.tools}]}}));
        let result = streams
            .find_thread(
                t.id as u64,
                &json!({"query":query,"start":start,"offset":offset}),
            )
            .and_then(|value| serde_json::from_value(value).map_err(|error| error.to_string()));
        self.apply_thread_find_result(req_id, result);
    }
    pub fn apply_thread_find_result(
        &mut self,
        req_id: u64,
        result: Result<crate::thread_find::ThreadFindResult, String>,
    ) {
        if !self.thread_find.open || req_id != self.thread_find.request_id {
            return;
        }
        self.thread_find.loading = false;
        match result {
            Ok(res) => {
                self.thread_find.error = None;
                self.thread_find.unsupported = false;
                self.thread_find.total_matches = res.total_matches;
                self.thread_find.active_index = res.active_index;
                self.thread_find.navigation = res.navigation;
                self.thread_find.active_match = res.active_match.clone();
                self.thread_find.pending_reveal = res.active_match;
            }
            Err(err) => {
                self.thread_find.error = Some(err);
            }
        }
    }
    pub fn consume_pending_find_reveal(&mut self, tx: &mut Tx) -> bool {
        if !self.thread_find_active() {
            return false;
        }
        let Some(matched) = self.thread_find.pending_reveal.clone() else {
            return false;
        };
        let t = self.current_mut();
        let plan_index = t.tools.iter().position(|tool| {
            tool.id == matched.entry_id && tool.resolved_kind() == crate::work::ToolKind::Plan
        });
        let msg_idx = t
            .messages
            .iter()
            .position(|m| m.id == matched.entry_id)
            .or_else(|| plan_index.map(|index| t.tools[index].anchor.message));
        let Some(msg_idx) = msg_idx else {
            if t.history_has_more && !t.history_loading && t.history_error.is_none() {
                let tid = t.id;
                self.load_earlier_history(tid, Some(matched.entry_id));
                return true;
            }
            return false;
        };
        let tid = t.id;
        let mut deferred_start = None;
        let row_key = if plan_index.is_some() || t.messages[msg_idx].role == "Pi" {
            let start = t.messages[..=msg_idx]
                .iter()
                .rposition(|m| m.role != "Pi")
                .map_or(0, |i| i + 1);
            let in_live_turn =
                t.status.busy() && t.messages[start..].iter().all(|m| m.role == "Pi");
            if in_live_turn && t.live_work_collapsed {
                t.live_work_collapsed = false;
                t.timeline_cache.get_mut().touch_message(start);
                t.timeline_cache.get_mut().touch_disclosure();
            }
            if let Some(work) = t.work.iter_mut().find(|w| w.start == start) {
                work.expanded = true;
            } else if plan_index.is_some() {
                t.work.push(AssistantWork {
                    start,
                    stopped: false,
                    stopped_order: 0,
                    expanded: true,
                    elapsed: None,
                    outcome: None,
                });
            }
            if let Some(index) = plan_index {
                t.tools[index].expanded = true;
            }
            if t.deferred_work
                .iter()
                .any(|marker| marker["start"] == start)
            {
                deferred_start = Some(start);
            }
            format!("turn-{tid}-{}", t.messages[start].id)
        } else {
            format!("row-{tid}-{}", t.messages[msg_idx].id)
        };
        if let Some(start) = deferred_start {
            self.load_deferred_work(start, false);
            // KSG holds find's reveal until the requested fold's history is mounted.
            return false;
        }
        self.thread_find.pending_reveal = None;
        // A long open turn is split into rows; reveal the one holding the match.
        let t = self.current();
        let query = self.thread_find.query.text.trim().to_owned();
        let mut seen = 0;
        let row_key = crate::ui::timeline_text_row(t, msg_idx, |chunk| {
            let hits = crate::thread_find::find_markdown_occurrences(chunk, &query).len();
            seen += hits;
            matched.occurrence < seen
        })
        .unwrap_or(row_key);
        tx.scroll_center_in_instant(
            format!("conversation-{tid}"),
            format!("{row_key}/thread-find-active"),
        );
        true
    }

    pub(crate) fn cite_assistant_selection(&mut self, tx: &mut Tx) -> bool {
        let Some(sel) = self.assistant_selection.clone() else {
            return false;
        };
        if self.demo {
            return false;
        }
        let current = self.current();
        if current.id != sel.thread_id
            || current.machine != sel.machine
            || current.archived()
            || current.request.is_some()
            || current.native.questionnaire.is_some()
        {
            return false;
        }
        let Some(message) = current
            .messages
            .iter()
            .find(|message| message.id == sel.message_id)
        else {
            return false;
        };
        if message.id != sel.message_id || message.role != "Pi" {
            return false;
        }
        let env_id = self.machine_identity(&current.machine);
        let thread_id_str = current.id.to_string();
        let Ok(citation) = crate::assistant_citation::build_assistant_citation_from_selection(
            &env_id,
            &thread_id_str,
            &message.id,
            &message.text,
            sel.chunk_from,
            &sel.text,
        ) else {
            return false;
        };
        let record =
            crate::assistant_citation::citation_context_record(citation, &message.id, None);
        let context_id = record.context_id.clone();
        // KSG opens the comment editor as soon as the chip lands; cancelling it
        // undoes the insert (`removeOnCancel` carries the spaces it added).
        let spaces = self.current_mut().insert_context_chip(record);
        if let Some(spaces) = spaces {
            crate::ui::citation_comment::open_after_insert(self, &context_id, spaces);
        }
        self.restore_composer_focus(crate::ui::composer_ui::FocusReason::Inserted);
        self.assistant_selection = None;
        tx.clear_text_selection();
        self.checkpoint();
        true
    }

    pub(crate) fn navigate_to_citation(
        &mut self,
        citation: &crate::composer_context::AssistantCitation,
        tx: &mut Tx,
    ) -> bool {
        let Ok(id) = citation.thread_id.parse::<usize>() else {
            return false;
        };
        let machine = if citation.environment_id == self.machine_identity("local") {
            "local".to_owned()
        } else {
            citation.environment_id.clone()
        };
        self.open_machine_thread(&machine, id);
        if self.current().id != id || self.machines.selected != machine {
            return false;
        }
        let t = self.current_mut();
        let msg_idx = t.messages.iter().position(|m| m.id == citation.message_id);
        let row_key = match msg_idx {
            Some(idx) if t.messages[idx].role == "Pi" => {
                let start = t.messages[..=idx]
                    .iter()
                    .rposition(|m| m.role != "Pi")
                    .map_or(0, |i| i + 1);
                let in_live_turn =
                    t.status.busy() && t.messages[start..].iter().all(|m| m.role == "Pi");
                if in_live_turn && t.live_work_collapsed {
                    t.live_work_collapsed = false;
                    t.timeline_cache.get_mut().touch_message(start);
                    t.timeline_cache.get_mut().touch_disclosure();
                }
                format!("turn-{id}-{}", t.messages[start].id)
            }
            _ => format!("row-{id}-{}", citation.message_id),
        };
        let t = self.current();
        // A long open turn is split into rows; scroll to the one holding the quote.
        let row_key = msg_idx
            .and_then(|message| {
                crate::ui::timeline_text_row(t, message, |chunk| {
                    !crate::thread_find::find_markdown_occurrences(chunk, citation.text.trim())
                        .is_empty()
                })
            })
            .unwrap_or(row_key);
        self.citation_highlight = Some(CitationHighlight {
            machine,
            thread_id: id,
            message_id: citation.message_id.clone(),
            text: citation.text.clone(),
        });
        tx.scroll_center_in(format!("conversation-{id}"), row_key);
        true
    }

    pub(crate) fn reply_to_message(&mut self, message_id: &str) -> bool {
        if self.demo {
            return false;
        }
        let current = self.current();
        if current.archived() || current.request.is_some() || current.native.questionnaire.is_some()
        {
            return false;
        }
        let Some(msg_idx) = current
            .messages
            .iter()
            .position(|m| m.id == message_id && m.role == "Pi" && !m.text.trim().is_empty())
        else {
            return false;
        };
        let message = &current.messages[msg_idx];
        let Some(opening) =
            crate::assistant_citation::capture_assistant_message_opening(&message.text)
        else {
            return false;
        };
        let env_id = self.machine_identity(&current.machine);
        let thread_id_str = current.id.to_string();
        let citation = crate::composer_context::AssistantCitation {
            version: 1,
            environment_id: env_id,
            thread_id: thread_id_str,
            message_id: message.id.clone(),
            text: opening.text,
            comment: None,
            start: opening.start,
            end: opening.end,
            prefix: opening.prefix,
            suffix: opening.suffix,
        };
        let subagents = if self.is_captain_home(current) {
            let turn_start = current.messages[..=msg_idx]
                .iter()
                .rposition(|m| m.role != "Pi")
                .map_or(0, |i| i + 1);
            let tasks = self.tasks_for(current.id);
            let mut refs = Vec::new();
            let mut seen_threads = std::collections::HashSet::new();
            for tool in &current.tools {
                if !(turn_start..=msg_idx).contains(&tool.anchor.message)
                    || (tool.name != "delegate_task" && !tool.name.ends_with("__delegate_task"))
                {
                    continue;
                }
                let child_id = serde_json::from_str::<Value>(&tool.output)
                    .ok()
                    .and_then(|v| v["thread_id"].as_u64())
                    .or_else(|| tool.structured["thread_id"].as_u64());
                if let Some(child_id) = child_id
                    && let Some(task) = tasks.iter().find(|task| task.child_id == child_id as usize)
                    && seen_threads.insert(task.child_id)
                {
                    refs.push(crate::composer_context::ResponseSubagentRef {
                        thread_id: task.child_id.to_string(),
                        title: task.title.clone(),
                        environment_id: task.remote.as_ref().map(|r| r.server_id.clone()),
                    });
                }
            }
            (!refs.is_empty()).then_some(refs)
        } else {
            None
        };
        let record =
            crate::assistant_citation::response_context_record(citation, &message.id, subagents);
        self.current_mut().draft_reply_to = Some(record);
        self.restore_composer_focus(crate::ui::composer_ui::FocusReason::Explicit);
        self.checkpoint();
        true
    }

    pub(crate) fn clear_reply_to(&mut self) {
        if self.current().draft_reply_to.is_some() {
            self.current_mut().draft_reply_to = None;
            self.checkpoint();
        }
    }
}
