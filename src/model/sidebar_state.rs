use super::*;

impl State {
    pub fn is_captain_home(&self, thread: &Thread) -> bool {
        thread.captain_home && !thread.delegated
    }
    pub fn thread_directory(&self, index: usize) -> Result<PathBuf, String> {
        let thread = self.threads.get(index).ok_or("Thread does not exist.")?;
        if self.client.is_some() && self.machines.selected != "local" {
            return self.client_views["directories"][thread.id.to_string()]
                .as_str()
                .map(PathBuf::from)
                .ok_or("The remote server has not reported this thread's host directory.".into());
        }
        crate::backend::BackendHost::resolve(&thread.machine)?;
        if let Some(worktree_path) = &thread.worktree_path {
            return Ok(PathBuf::from(worktree_path));
        }
        match thread.project {
            Some(id) => {
                let project = self
                    .project(Some(id))
                    .ok_or("Thread project does not exist.")?;
                if project.machine != thread.machine {
                    return Err("Thread and project machines do not match.".into());
                }
                Ok(project.path.clone())
            }
            None if thread.scratch_directory.is_some() => {
                Ok(PathBuf::from(thread.scratch_directory.as_ref().unwrap()))
            }
            None if thread.captain_home => Ok(self
                .data_dir
                .join("captains")
                .join(
                    thread
                        .captain
                        .ok_or("Captain home has no profile.")?
                        .to_string(),
                )
                .join("scratch")),
            None => Ok(self.data_dir.join("chats").join(thread.id.to_string())),
        }
    }
    pub fn working_directory(&self) -> PathBuf {
        self.thread_directory(self.active)
            .unwrap_or_else(|_| self.data_dir.clone())
    }
    pub fn sorted_projects(&self) -> Vec<ProjectId> {
        let mut projects: Vec<_> = self
            .projects
            .entries
            .iter()
            .filter(|p| !p.hidden || self.show_archive)
            .collect();
        projects.sort_by_key(|p| {
            std::cmp::Reverse((
                self.threads
                    .iter()
                    .filter(|t| t.project == Some(p.id))
                    .map(|t| t.updated)
                    .max()
                    .unwrap_or(p.updated)
                    .max(p.updated),
                p.id,
            ))
        });
        projects.into_iter().map(|p| p.id).collect()
    }
    pub(crate) fn hash_sidebar_coordinators(&self, hash: &mut impl std::hash::Hasher) {
        use std::hash::Hash as _;
        for task in self
            .tasks
            .iter()
            .filter(|task| task.coordinator_captain.is_some())
        {
            task.child_thread_id.hash(hash);
        }
    }

    pub(crate) fn sidebar_thread_candidates(&self) -> impl Iterator<Item = (usize, &Thread)> {
        let standalone: std::collections::HashSet<_> = self
            .threads
            .iter()
            .filter(|thread| !thread.captain_home)
            .map(|thread| thread.id)
            .collect();
        let coordinated: std::collections::HashSet<_> = self
            .tasks
            .iter()
            .filter(|task| task.coordinator_captain.is_some())
            .map(|task| task.child_thread_id)
            .collect();
        self.threads
            .iter()
            .enumerate()
            .filter(move |(_, t)| {
                t.archived() == self.show_archive
                    && t.crew_captain.is_none()
                    && (!t.delegated
                        || t.parent.is_some_and(|id| standalone.contains(&id))
                        || coordinated.contains(&t.id))
                    && !self.is_captain_home(t)
                    && (!t.is_draft() || self.draft_listed(t))
            })
            .filter(|(_, t)| self.in_scope(t))
    }

    pub fn sidebar_threads(&self) -> Vec<usize> {
        let mut threads: Vec<_> = self.sidebar_thread_candidates().collect();
        let at = lifecycle::now();
        threads.sort_by(|(_, a), (_, b)| {
            let sa = self.thread_section(a, at);
            let sb = self.thread_section(b, at);
            self.draft_listed(b)
                .cmp(&self.draft_listed(a))
                .then_with(|| sa.cmp(&sb))
                .then_with(|| {
                    lifecycle::compare(
                        sa,
                        &a.life,
                        &b.life,
                        a.working.returned_at,
                        b.working.returned_at,
                    )
                })
                .then_with(|| a.id.cmp(&b.id))
                .then_with(|| a.machine.cmp(&b.machine))
        });
        threads.into_iter().map(|(i, _)| i).collect()
    }
    /// Coordinators share the same workspace and search scope as standalone threads.
    /// In the archive, keep a home visible when it owns archived worker chats.
    pub fn sidebar_captains(&self) -> Vec<usize> {
        self.threads
            .iter()
            .enumerate()
            .filter(|(_, home)| {
                self.is_captain_home(home)
                    && home
                        .captain
                        .is_some_and(|captain| self.captain_home(captain) == Some(home.id))
                    && (self.in_scope(home)
                        || home.captain.is_some_and(|captain| {
                            self.captain_scope(captain)
                                .iter()
                                .any(|id| self.scope_includes(Some(*id)))
                        }))
            })
            .filter(|(_, home)| {
                let children = || {
                    self.threads.iter().filter(|child| {
                        self.captain_worker(child, home.id) && child.archived() == self.show_archive
                    })
                };
                let own_match =
                    home.archived() == self.show_archive && self.thread_search_matches(home);
                if self.show_archive {
                    own_match
                        || children().any(|child| {
                            self.thread_search_matches(home) || self.thread_search_matches(child)
                        })
                } else {
                    !home.archived()
                        && (own_match || children().any(|child| self.thread_search_matches(child)))
                }
            })
            .map(|(index, _)| index)
            .collect()
    }
    pub fn sync_sidebar_search_input(&mut self) -> bool {
        // Rune exposes text edits by value, not a content revision. Compare the
        // original input before trimming or folding it on a background tick.
        if self.search.text == self.sidebar_search.input_text {
            return false;
        }
        self.sidebar_search.input_text.clone_from(&self.search.text);
        let clipped: String = self
            .search
            .text
            .trim()
            .chars()
            .take(crate::thread_search::THREAD_SEARCH_QUERY_MAX_LENGTH)
            .collect();
        let trimmed = clipped.trim().to_owned();
        if trimmed == self.sidebar_search.last_typed {
            return false;
        }
        self.sidebar_search.last_typed = trimmed.clone();
        self.sidebar_search.folded_query = trimmed.to_lowercase();
        self.sidebar_search.request_seq += 1;
        self.sidebar_search.pending_servers.clear();
        self.sidebar_search.unsupported_servers.clear();
        self.sidebar_search.matches.clear();
        self.sidebar_search.settled_query.clear();
        self.sidebar_search.visible_query.clear();
        self.sidebar_search.visible_matches.clear();
        self.sidebar_search.shown_order.clear();
        let tokens = crate::thread_search::message_search_tokens(&trimmed);
        self.sidebar_search.pending = self.client.is_some() && !tokens.long.is_empty();
        if tokens.long.is_empty() {
            self.sidebar_search.debounce_until = None;
            return true;
        }
        if self.client.is_some() {
            self.sidebar_search.debounce_until = Some(
                Instant::now()
                    + Duration::from_millis(crate::thread_search::THREAD_SEARCH_DEBOUNCE_MS),
            );
        } else {
            self.sidebar_search.debounce_until = None;
            self.run_sidebar_search(&trimmed);
        }
        true
    }
    pub fn poll_sidebar_search(&mut self, tx: &mut Tx) -> bool {
        let mut changed = self.sync_sidebar_search_input();
        if let Some(until) = self.sidebar_search.debounce_until {
            let now = Instant::now();
            if now >= until {
                self.sidebar_search.debounce_until = None;
                let q = self.sidebar_search.last_typed.clone();
                self.run_sidebar_search(&q);
                changed = true;
            } else {
                tx.wake_after(until.saturating_duration_since(now));
            }
        }
        changed
    }
    pub(super) fn run_sidebar_search(&mut self, query: &str) {
        let tokens = crate::thread_search::message_search_tokens(query);
        if tokens.long.is_empty() {
            return;
        }
        self.sidebar_search.request_seq += 1;
        let seq = self.sidebar_search.request_seq;
        self.sidebar_search.settled_query = query.to_owned();
        self.sidebar_search.matches.clear();
        self.sidebar_search.pending_servers.clear();
        self.sidebar_search.unsupported_servers.clear();
        let Some(client) = &self.client else {
            // Keep visible_query/visible_matches in non-server mode if manually seeded in tests;
            // local message matching runs in thread_title_search_matches.
            return;
        };
        let mut targets: Vec<(String, &crate::client::Client)> = vec![("local".into(), client)];
        for (id, linked) in &client.linked {
            targets.push((id.clone(), linked));
        }
        for (server, target) in targets {
            if !target.connected_ready() {
                continue;
            }
            if !target.has_capability(crate::protocol::THREAD_SEARCH) {
                self.sidebar_search
                    .unsupported_servers
                    .push(self.machine_label(&server));
                continue;
            }
            self.sidebar_search.pending_servers.insert(server.clone());
            target.call_rpc(
                format!("search:{seq}:{server}"),
                "threads.search",
                json!({"query": query, "limit": 50}),
            );
        }
        self.sidebar_search.pending = !self.sidebar_search.pending_servers.is_empty();
    }
    pub fn apply_sidebar_search_result(
        &mut self,
        seq: u64,
        server: &str,
        result: Result<Vec<crate::thread_search::ThreadSearchHit>, String>,
    ) {
        self.sync_sidebar_search_input();
        if seq != self.sidebar_search.request_seq
            || self.sidebar_search.settled_query != self.sidebar_search.last_typed
            || !self.sidebar_search.pending_servers.remove(server)
        {
            return;
        }
        self.sidebar_search.pending = !self.sidebar_search.pending_servers.is_empty();
        if let Ok(hits) = result {
            self.sidebar_search.matches.insert(server.to_owned(), hits);
            // Rank across all responding environments and preserve shown order (KSG nextShownOrder).
            let mut combined: Vec<(String, crate::thread_search::ThreadSearchHit)> = self
                .sidebar_search
                .matches
                .iter()
                .flat_map(|(srv, list)| list.iter().map(move |h| (srv.clone(), h.clone())))
                .collect();
            let weight = |hits: usize| 1.0 + (1.0 + hits as f64).log2();
            combined.sort_by(|(_, left), (_, right)| {
                let lw = left.quality >= crate::thread_search::MESSAGE_PREFIX;
                let rw = right.quality >= crate::thread_search::MESSAGE_PREFIX;
                rw.cmp(&lw)
                    .then_with(|| {
                        right
                            .quality
                            .partial_cmp(&left.quality)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .then_with(|| {
                        weight(right.hits)
                            .partial_cmp(&weight(left.hits))
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .then_with(|| right.updated.cmp(&left.updated))
            });
            let ranked_keys: Vec<(String, usize)> = combined
                .iter()
                .map(|(srv, hit)| (srv.clone(), hit.thread_id))
                .collect();
            let ranked_set: std::collections::HashSet<(String, usize)> =
                ranked_keys.iter().cloned().collect();
            let same_query = self.sidebar_search.visible_query == self.sidebar_search.settled_query;
            let mut next_order: Vec<(String, usize)> = if same_query {
                self.sidebar_search
                    .shown_order
                    .iter()
                    .filter(|k| ranked_set.contains(*k))
                    .cloned()
                    .collect()
            } else {
                Vec::new()
            };
            let held: std::collections::HashSet<(String, usize)> =
                next_order.iter().cloned().collect();
            for key in ranked_keys {
                if !held.contains(&key) {
                    next_order.push(key);
                }
            }
            self.sidebar_search.shown_order = next_order;
            self.sidebar_search.visible_query = self.sidebar_search.settled_query.clone();
            self.sidebar_search.visible_matches = combined
                .into_iter()
                .map(|(srv, hit)| ((srv, hit.thread_id), hit))
                .collect();
        }
    }
    pub fn open_thread_at_search_match(&mut self, server: &str, id: usize) {
        let hit = self
            .sidebar_search
            .visible_matches
            .get(&(server.to_owned(), id))
            .cloned();
        let query = if self.sidebar_search.visible_query.is_empty() {
            self.search.text.trim().to_owned()
        } else {
            self.sidebar_search.visible_query.clone()
        };
        if self.client.is_some() {
            self.open_machine_thread(server, id);
        } else if let Some(index) = self.threads.iter().position(|t| t.id == id) {
            self.show_thread(index);
        }
        if let Some(hit) = hit
            && self.current().id == id
        {
            let (find_query, start) =
                crate::thread_search::find_for_message_match(&hit.snippet, &hit.entry_id, &query);
            self.open_thread_find_on_query(find_query, start);
        }
    }
    pub fn thread_title_search_matches(&self, thread: &Thread) -> bool {
        if self.search.text == self.sidebar_search.input_text {
            return self.thread_title_matches_folded(thread, &self.sidebar_search.folded_query);
        }
        self.thread_title_matches_query(thread, &self.search.text)
    }
    /// The title match the sidebar's search and the quick switcher share:
    /// title, a listed draft's text, the captain's name on a home, and (only
    /// where the server cannot search bodies) message text.
    pub fn thread_title_matches_query(&self, thread: &Thread, search: &str) -> bool {
        let search = search.trim().to_lowercase();
        self.thread_title_matches_folded(thread, &search)
    }
    pub(super) fn thread_title_matches_folded(&self, thread: &Thread, search: &str) -> bool {
        if search.is_empty() {
            return true;
        }
        let title_or_draft = thread.title.to_lowercase().contains(search)
            || self.draft_listed(thread) && thread.draft.text.to_lowercase().contains(search)
            || self.is_captain_home(thread)
                && thread.captain.is_some_and(|captain| {
                    self.captains[captain].name.to_lowercase().contains(search)
                });
        if title_or_draft {
            return true;
        }
        let server_supports = self.client.as_ref().is_some_and(|c| {
            let target = if self.machines.selected == "local" {
                Some(c)
            } else {
                c.linked.get(&self.machines.selected)
            };
            target.is_some_and(|t| t.has_capability(crate::protocol::THREAD_SEARCH))
        });
        if !server_supports {
            return thread
                .messages
                .iter()
                .any(|message| message.text.to_lowercase().contains(search));
        }
        false
    }
    pub fn thread_search_matches(&self, thread: &Thread) -> bool {
        if self.search.text.is_empty() {
            return true;
        }
        self.thread_title_search_matches(thread)
            || self
                .sidebar_search
                .visible_matches
                .contains_key(&(self.machines.selected.clone(), thread.id))
    }
    /// Crew workers that match the current search or archive filter. They list as flat
    /// ordinary rows under their own match, not nested under the captain. With no query and
    /// the archive closed, workers never enter the thread list.
    pub fn searched_workers(&self) -> Vec<usize> {
        if self.search.text.is_empty() && !self.show_archive {
            return vec![];
        }
        self.threads
            .iter()
            .enumerate()
            .filter(|(_, child)| {
                child.crew_captain.is_some()
                    && child.archived() == self.show_archive
                    && self.in_scope(child)
                    && self.thread_search_matches(child)
            })
            .map(|(index, _)| index)
            .collect()
    }
    /// Keyboard traversal follows the rows that the sidebar actually displays.
    #[cfg(test)]
    pub fn sidebar_navigation(&self) -> Vec<usize> {
        let mut rows: Vec<usize> = self.sidebar_captains();
        // Crew workers list flat during search and in the archive. The sidebar's
        // own ordering (sidebar_ui::entries) interleaves them with ordinary rows,
        // so traversal must reproduce that interleave rather than append them.
        let mut ordinary: Vec<usize> = self.visible_sidebar_threads();
        ordinary.extend(self.searched_workers());
        let at = lifecycle::now();
        let searching = !self.search.text.is_empty();
        let title_match = |t: &Thread| !searching || self.thread_title_search_matches(t);
        let order_pos = |index: usize| -> usize {
            self.sidebar_search
                .shown_order
                .iter()
                .position(|(server, id)| {
                    server == &self.machines.selected && *id == self.threads[index].id
                })
                .unwrap_or(usize::MAX)
        };
        ordinary.sort_by(|&a, &b| {
            let (ta, tb) = (&self.threads[a], &self.threads[b]);
            let ordinary_rank = |state: &State| {
                state
                    .draft_listed(tb)
                    .cmp(&state.draft_listed(ta))
                    .then_with(|| {
                        let sa = state.thread_section(ta, at);
                        let sb = state.thread_section(tb, at);
                        sa.cmp(&sb)
                            .then_with(|| {
                                lifecycle::compare(
                                    sa,
                                    &ta.life,
                                    &tb.life,
                                    ta.working.returned_at,
                                    tb.working.returned_at,
                                )
                            })
                            .then(ta.id.cmp(&tb.id))
                    })
            };
            if searching {
                title_match(tb).cmp(&title_match(ta)).then_with(|| {
                    if !title_match(ta) && !title_match(tb) {
                        order_pos(a).cmp(&order_pos(b)).then(ta.id.cmp(&tb.id))
                    } else {
                        ordinary_rank(self)
                    }
                })
            } else {
                ordinary_rank(self)
            }
        });
        rows.extend(ordinary);
        rows
    }
    /// Whether an unsent draft has a "Drafts" row. Other drafts need content. The open draft keeps
    /// whatever it had when it was opened, so its row does not appear or vanish while typing.
    pub fn draft_listed(&self, t: &Thread) -> bool {
        if !t.is_draft() {
            return false;
        }
        match self.opened_draft {
            Some((id, listed)) if id == t.id && id == self.current().id => listed,
            _ => t.has_draft_content() || t.renamed,
        }
    }
    /// Records whether the open thread is a listed draft as it is opened or restored.
    pub(super) fn capture_opened_draft(&mut self) {
        let t = self.current();
        let listed = t.is_draft() && (t.has_draft_content() || t.renamed);
        self.opened_draft = Some((t.id, listed));
    }
    pub fn in_scope(&self, t: &Thread) -> bool {
        self.scope_includes(t.project)
    }
    /// Whether the workspace filter shows this workspace (None is No workspace).
    pub fn scope_includes(&self, project: Option<ProjectId>) -> bool {
        self.scope_includes_on(&self.machines.selected, project)
    }
    pub fn scope_includes_on(&self, server: &str, project: Option<ProjectId>) -> bool {
        let owner = self
            .machines
            .scope_server
            .as_deref()
            .unwrap_or(&self.machines.selected);
        let same_server = self.client.is_none() || server == owner;
        let additional = project.is_some_and(|p| {
            same_server && self.scope_additional.contains(&p)
                || self
                    .machines
                    .scope_remote
                    .iter()
                    .any(|(owner, id)| owner == server && *id == p)
        });
        match self.project_scope {
            ProjectScope::All => true,
            ProjectScope::Project(id) => same_server && project == Some(id) || additional,
            ProjectScope::NoProject => project.is_none() || additional,
        }
    }
    pub fn set_project_scope(&mut self, project: Option<ProjectId>, multi: bool) {
        self.set_machine_scope(self.machines.selected.clone(), project, multi);
    }
    pub fn set_machine_scope(&mut self, server: String, project: Option<ProjectId>, multi: bool) {
        let owner = self
            .machines
            .scope_server
            .clone()
            .unwrap_or_else(|| self.machines.selected.clone());
        // None means the global "No workspace" choice; every project has an
        // explicit server. Keep the legacy primary/additional fields readable.
        let mut choices: Vec<Option<(String, usize)>> = if multi {
            match self.project_scope {
                ProjectScope::All => vec![],
                ProjectScope::NoProject => std::iter::once(None)
                    .chain(
                        self.scope_additional
                            .iter()
                            .map(|id| Some((owner.clone(), *id))),
                    )
                    .chain(self.machines.scope_remote.iter().cloned().map(Some))
                    .collect(),
                ProjectScope::Project(id) => std::iter::once(Some((owner.clone(), id)))
                    .chain(
                        self.scope_additional
                            .iter()
                            .map(|id| Some((owner.clone(), *id))),
                    )
                    .chain(self.machines.scope_remote.iter().cloned().map(Some))
                    .collect(),
            }
        } else {
            vec![]
        };
        let target = project.map(|id| (server, id));
        if let Some(i) = choices.iter().position(|choice| choice == &target) {
            choices.remove(i);
        } else {
            choices.push(target);
        }
        choices.sort();
        choices.dedup();
        self.project_scope = match choices.first() {
            None => ProjectScope::All,
            Some(None) => ProjectScope::NoProject,
            Some(Some((_, id))) => ProjectScope::Project(*id),
        };
        let owner = choices
            .iter()
            .flatten()
            .next()
            .map(|(server, _)| server.clone());
        self.machines.scope_server = owner.clone();
        self.scope_additional.clear();
        self.machines.scope_remote.clear();
        for (server, id) in choices.into_iter().skip(1).flatten() {
            if Some(&server) == owner.as_ref() {
                self.scope_additional.push(id);
            } else {
                self.machines.scope_remote.push((server, id));
            }
        }
        self.selected_threads.clear();
        if !multi {
            self.project_picker = None;
        }
        self.checkpoint();
    }
    /// Open the workspace picker with an empty search.
    pub fn open_picker(&mut self, mode: ProjectPicker) {
        if mode == ProjectPicker::AddProject {
            if self.project_picker != Some(ProjectPicker::AddProject) {
                self.projects.return_picker = self.project_picker;
            }
            self.projects.browser = Default::default();
        } else {
            self.machines.draft_picker = None;
        }
        if self.captain_editor.is_none() {
            self.cancel_captain_flow();
        }
        self.picker_browsing = false;
        self.project_picker = Some(mode);
        self.picker_query = TextEdit::default();
        self.picker_query.focused = true;
        self.picker_highlight = 0;
    }
    /// The quick switcher (`commandPalette.toggle`, `mod+k`, or the bar's title
    /// button). Focus lands in its search field, as in KSG Code's CommandPalette.
    pub fn open_quick_switcher(&mut self) {
        self.quick_switcher = true;
        self.quick_switcher_query = TextEdit::default();
        self.quick_switcher_query.focused = true;
        self.quick_switcher_highlight = 0;
    }
    /// Initial query for the Add workspace folder browser, ported from KSG Code's
    /// `getAddProjectInitialQueryForEnvironment`: uses `addProjectBaseDirectory` from the
    /// target machine's thread defaults (ensuring a trailing separator), or `"~/"` when empty.
    pub fn add_project_initial_query(&self) -> String {
        let base = if self.client.is_none() || self.machines.selected == "local" {
            self.thread_defaults.add_project_base_directory.trim()
        } else {
            self.machines
                .views
                .get(&self.machines.selected)
                .and_then(|v| v["index"]["threadDefaults"]["add_project_base_directory"].as_str())
                .map(str::trim)
                .unwrap_or("")
        };
        if base.is_empty() {
            return "~/".into();
        }
        let mut out = base.to_string();
        if !out.ends_with(['/', '\\']) {
            out.push(if out.contains('\\') && !out.contains('/') {
                '\\'
            } else {
                '/'
            });
        }
        out
    }
    /// Switch the Add workspace palette to browsing the selected server user's home or
    /// configured "Add project starts in" directory.
    /// Ported from KSG Code's `startAddProjectBrowse`.
    pub fn browse_for_project(&mut self) {
        self.project_picker = Some(ProjectPicker::AddProject);
        self.picker_browsing = true;
        self.projects.browser = Default::default();
        self.picker_query = TextEdit::default();
        // The answering host resolves home/base. Do not derive remote parents on this machine.
        let initial = self.add_project_initial_query();
        self.picker_query.replace_text(&initial);
        self.picker_query.focused = true;
        self.picker_highlight = 0;
    }
    /// Add the typed folder, creating it first when it does not exist yet.
    pub fn add_browsed_project(&mut self) {
        let text = if self.picker_query.text.is_empty() {
            self.projects
                .browser
                .listing
                .as_ref()
                .map(|listing| listing.path.clone())
                .unwrap_or_default()
        } else {
            self.picker_query.text.clone()
        };
        if text.is_empty() {
            return;
        }
        if self.client.is_some() {
            let (purpose, captain) = self.project_selection_purpose();
            let current_listing = self.projects.browser.request.as_ref()
                == Some(&(
                    self.machines.selected.clone(),
                    self.picker_query.text.clone(),
                ))
                && self.projects.browser.pending.is_none();
            let creates = self
                .projects
                .browser
                .listing
                .as_ref()
                .is_some_and(|listing| !listing.target_exists)
                || self.projects.browser.error.as_deref() == Some("That folder does not exist.");
            let creates = current_listing && creates;
            match self.remote("project.put", json!({"path":text,"createFolder":creates,"selectFor":purpose,"captain":captain})).expect("client") {
                Ok(_) => {
                    self.project_picker = None;
                    self.picker_browsing = false;
                    self.projects.menu_error = None;
                }
                Err(error) => self.projects.menu_error = Some(error),
            }
            return;
        }
        let path = crate::projects::expand_path(&text);
        if !path.exists()
            && let Err(error) = std::fs::create_dir_all(&path)
        {
            self.projects.menu_error = Some(format!("Could not create that folder: {error}"));
            return;
        }
        match self.add_project(path) {
            Ok(_) => {
                self.project_picker = None;
                self.picker_browsing = false;
                self.projects.menu_error = None;
                self.checkpoint();
            }
            Err(error) => self.projects.menu_error = Some(error),
        }
    }
    /// The sidebar's New thread, ported from KSG Code's `shouldCreateNewThreadInCurrentProject`
    /// and `startNewThreadFromContext`: with one workspace in view, no workspaces at all, or
    /// Shift-click in a visible workspace, start at once; otherwise open the New thread picker.
    pub fn new_thread_click(&mut self, shift: bool) {
        let shown = |s: &Self, project: Option<ProjectId>| {
            s.scope_includes(project)
                && project.is_none_or(|id| s.project(Some(id)).is_some_and(|p| !p.hidden))
        };
        let in_view: Vec<_> = self
            .projects
            .entries
            .iter()
            .filter(|p| shown(self, Some(p.id)))
            .map(|p| p.id)
            .collect();
        let all_visible_empty = self.projects.entries.iter().all(|p| p.hidden);
        let current = self.current().project;
        let target = if shift && current.is_some() && shown(self, current) {
            Some(current)
        } else if in_view.len() == 1 {
            Some(Some(in_view[0]))
        } else if all_visible_empty && in_view.is_empty() {
            Some(None)
        } else {
            None
        };
        match target {
            Some(project) => {
                self.new_thread_in(project);
                self.restore_composer_focus(crate::ui::composer_ui::FocusReason::Explicit);
            }
            None => self.open_picker(ProjectPicker::NewThread),
        }
    }
    #[cfg(test)]
    pub fn click_thread(&mut self, index: usize, mods: rune::Mods) {
        let visible_ids: Vec<usize> = self
            .visible_sidebar_threads()
            .into_iter()
            .map(|i| self.threads[i].id)
            .collect();
        self.click_thread_in_order(index, mods, &visible_ids);
    }
    pub fn click_thread_in_order(&mut self, index: usize, mods: rune::Mods, visible_ids: &[usize]) {
        let id = self.threads[index].id;
        if mods.cmd {
            if let Some(i) = self.selected_threads.iter().position(|t| *t == id) {
                self.selected_threads.remove(i);
            } else {
                self.selected_threads.push(id);
            }
        } else if mods.shift {
            let start = visible_ids
                .iter()
                .position(|tid| Some(*tid) == self.selection_anchor);
            let end = visible_ids.iter().position(|tid| *tid == id);
            if let (Some(a), Some(b)) = (start, end) {
                self.selected_threads = visible_ids[a.min(b)..=a.max(b)].to_vec();
            }
        } else {
            self.show_thread(index);
        }
        if !mods.shift {
            self.selection_anchor = Some(id);
        }
    }
    pub fn thread_section(&self, t: &Thread, at: u64) -> Section {
        if t.machine != "local" {
            return if t.pinned() {
                Section::Pinned
            } else {
                Section::Active
            };
        }
        // KSG's `isThreadWorking`: a running thread leaves the inbox for the Working
        // shelf, but a thread waiting on the user stays where it is.
        lifecycle::section(
            &t.life,
            at,
            t.working.was_working || t.status == Status::Running && !self.row_pending(t),
        )
    }
    pub(super) fn connection_instructions(&self, index: usize) -> Option<String> {
        if self.threads[index].crew_captain.is_some() {
            let prompt = self.threads[index].memory_prompt();
            (!prompt.is_empty()).then_some(prompt)
        } else {
            self.threads[index]
                .captain
                .and_then(|id| self.captains.get(id))
                .map(|captain| {
                    format!(
                        "{}\n\n{}",
                        captain.prompt(),
                        self.threads[index].memory_prompt()
                    )
                })
        }
    }
    pub fn row_pending(&self, t: &Thread) -> bool {
        t.request.is_some()
            || t.status == Status::Waiting
            || self.threads.iter().any(|c| {
                c.parent == Some(t.id)
                    && c.request.as_ref().is_some_and(|r| r["method"] == "confirm")
            })
    }
    pub fn selected_row(&self) -> usize {
        let current = self.current();
        if let Some(captain) = current.crew_captain {
            return self.captain_home(captain).unwrap_or(current.id);
        }
        if current.delegated
            && self.tasks.iter().any(|task| {
                task.child_thread_id == current.id && task.coordinator_captain.is_some()
            })
        {
            return current.id;
        }
        current
            .parent
            .map(|parent| self.effective_home(parent))
            .unwrap_or(current.id)
    }
    #[cfg(test)]
    pub fn visible_sidebar_threads(&self) -> Vec<usize> {
        let mut settled = 0;
        self.sidebar_threads()
            .into_iter()
            .filter(|i| {
                let t = &self.threads[*i];
                if !self.search.text.is_empty() {
                    return self.thread_search_matches(t);
                }
                let section = self.thread_section(t, lifecycle::now());
                let shelf = match section {
                    Section::Working => Some(0),
                    Section::Snoozed => Some(1),
                    Section::Settled => Some(2),
                    _ => None,
                };
                if let Some(shelf) = shelf {
                    if !self.shelves[shelf] {
                        return t.id == self.selected_row();
                    }
                    if section == Section::Settled {
                        settled += 1;
                        return settled <= self.settled_visible || t.id == self.selected_row();
                    }
                }
                true
            })
            .collect()
    }
    pub fn drag_pointer(&mut self, id: usize, drag: &rune::Drag) {
        if drag.cancelled() {
            self.drag_thread = None;
            self.drag_target = None;
            return;
        }
        if drag.dx.hypot(drag.dy) >= 6.0 && drag.phase != rune::Phase::Up {
            self.drag_thread = Some(id);
        }
        if self.drag_thread == Some(id) {
            let [x, y] = drag.window_point();
            self.drag_target = crate::ui::sidebar_ui::drag_row_at(self, id, [x, y]);
            for section in [Section::Pinned, Section::Active, Section::Settled] {
                if let Some([rx, ry, rw, rh]) =
                    self.drop_rects.get(&format!("drop-{section:?}")).copied()
                    && x >= rx
                    && x <= rx + rw
                    && y >= ry
                    && y <= ry + rh
                {
                    self.drag_target = Some((section, None));
                }
            }
        }
        if drag.phase == rune::Phase::Up {
            if let Some((section, before)) = self.drag_target.take() {
                self.drop_thread(id, section, before);
            }
            self.drag_thread = None;
        }
    }
}
