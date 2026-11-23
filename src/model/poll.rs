use super::*;

/// Ported from KSG Code's `isClearComposerCommand` (`packages/shared/src/composerTrigger.ts`).
pub fn is_clear_command(text: &str) -> bool {
    text.trim().eq_ignore_ascii_case("/clear")
}

/// Ported from KSG Code's `COMPACT_COMMAND_PATTERN` (`packages/client-runtime/src/state/contextWindow.ts`).
pub fn is_compact_command(text: &str) -> bool {
    let trimmed = text.trim_start();
    trimmed
        .get(..8)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("/compact"))
        && trimmed[8..].chars().next().is_none_or(char::is_whitespace)
}

pub fn poll(state: &mut State, now: f64, tx: &mut Tx) -> bool {
    state.expire_attachment_preview();
    let attachment_changed = state.poll_attachment_uploads();
    if state.attachment_uploads.entries.iter().any(|entry| {
        matches!(
            entry.phase,
            crate::attachment_uploads::Phase::Preparing
                | crate::attachment_uploads::Phase::Uploading { .. }
        )
    }) {
        tx.wake_after(Duration::from_millis(30));
    }
    let composer_changed = !state.headless && crate::ui::composer_ui::tick(state, now, tx);
    let mut html_changed = false;
    while let Some(outcome) = state.html_windows.next() {
        html_changed = true;
        match outcome {
            Ok(Some(url)) => {
                let _ = tx.open_url(&url);
            }
            Ok(None) => {}
            Err(error) => state.storage_error = Some(error),
        }
    }
    state.html_windows.theme(state.dark);
    for url in std::mem::take(&mut state.pending_open_urls) {
        html_changed = true;
        if let Err(error) = tx.open_url(&url) {
            state.push_toast(
                ToastKind::Error,
                "Could not open the link",
                Some(error.to_string()),
                None,
                Some(5_000),
            );
        }
    }
    let keybindings_changed = !state.headless && state.poll_keybindings();
    let mut browsing = state.poll_folder_browser()
        | state.poll_settings()
        | state.poll_thread_find(tx)
        | state.poll_sidebar_search(tx)
        | crate::ui::composer_menu::poll_mention_search(state, tx)
        | html_changed
        | attachment_changed
        | keybindings_changed;
    browsing |= crate::ui::sidebar_preview::poll(state, tx);
    if let Some(error) = state.lifecycle_error.take() {
        state.push_toast(
            ToastKind::Error,
            "Could not change the thread",
            Some(error),
            None,
            Some(5_000),
        );
        browsing = true;
    }
    if let Some(row) = state.pending_tool_reveal.take() {
        tx.scroll_center_in_instant(format!("conversation-{}", state.current().id), row);
        browsing = true;
    }
    if let Some((_, started)) = &state.flash_tool {
        let remaining = Duration::from_secs(2).saturating_sub(started.elapsed());
        if remaining.is_zero() {
            if let Some((id, _)) = state.flash_tool.take() {
                let thread = state.current_mut();
                if let Some(tool) = thread.tools.iter().find(|tool| tool.id == id) {
                    thread
                        .timeline_cache
                        .get_mut()
                        .touch_message(tool.anchor.message);
                }
            }
            browsing = true;
        } else {
            tx.wake_after(remaining);
        }
    }
    // Linked-machine examples are offline fixtures. Animate their artwork, but
    // never run native provider maintenance or client commands for those records.
    if state.demo && state.client.is_some() {
        if let Some(wait) = crate::ui::tick_portraits(state, Instant::now()) {
            tx.wake_after(wait);
            return true;
        }
        return browsing | composer_changed;
    }
    let availability_changed = state.poll_provider_availability();
    if state.client.is_some() {
        return state.poll_client(tx)
            | crate::ui::diff_panel::tick(state, tx)
            | browsing
            | availability_changed
            | composer_changed;
    }
    let browsing = browsing | state.poll_fleet();
    let background_changed = state.poll_background_start(tx);
    if state.background.quitting {
        return true;
    }
    if let Some(marker) = &mut state.run_marker {
        if let Err(error) = marker.tick() {
            state.storage_error = Some(error);
        }
        tx.wake_after(Duration::from_secs(60));
    }
    // A resumed event loop is no longer committed to the earlier quit attempt.
    if state.restart_approved && !rune::quitting() {
        state.cancel_update_exit();
    }
    let mut changed = browsing
        | composer_changed
        | availability_changed
        | background_changed
        | state.updater.poll(Instant::now(), state.wake.clone());
    if let Some(remaining) = state.updater.remaining(Instant::now()) {
        tx.wake_after(remaining);
    }
    changed |= state.poll_projects();
    for thread in &mut state.threads {
        if !thread.draft.focused {
            thread.typing_guard.on_blur_or_send();
        }
    }
    if state.catalog.poll() {
        state.apply_catalog();
        changed = true;
    }
    if let Some(remaining) = state.catalog.remaining() {
        tx.wake_after(remaining);
    }
    changed |= state.refresh_native_catalogs(false, true);
    let mut acp_changed = false;
    for (harness, catalog) in &mut state.acp_catalogs {
        acp_changed |= catalog.poll(harness);
    }
    // KSG `refreshWorkspaceSnapshot` results: a completed per-folder scan lands
    // on the canonical catalog's workspace_snapshots, then the scan is dropped.
    let scan_keys: Vec<_> = state.workspace_model_scans.keys().cloned().collect();
    for (harness, cwd) in scan_keys {
        let key = (harness.clone(), cwd.clone());
        let (changed, done) = {
            let Some(scan) = state.workspace_model_scans.get_mut(&key) else {
                continue;
            };
            (scan.poll(&harness), scan.commands_ready())
        };
        acp_changed |= changed;
        if !done {
            continue;
        }
        let scan = state.workspace_model_scans.remove(&key).unwrap();
        if scan.error.is_none()
            && let Some(catalog) = state.native_catalog_mut(&harness)
        {
            catalog.set_workspace_models(&cwd, &scan.models);
        }
    }
    if state.codex_catalog.poll("codex")
        | state.claude_catalog.poll("claude")
        | state.antigravity_catalog.poll("antigravity")
        | state.cursor_catalog.poll("cursor")
        | acp_changed
    {
        state.apply_native_catalogs();
        changed = true;
    }
    if state.codex_catalog.loading()
        || state.claude_catalog.loading()
        || state.antigravity_catalog.loading()
        || state.cursor_catalog.loading()
        || state
            .acp_catalogs
            .values()
            .any(crate::session::NativeCatalog::loading)
        || state
            .workspace_model_scans
            .values()
            .any(crate::session::NativeCatalog::loading)
    {
        tx.wake_after(Duration::from_millis(50));
    }
    changed |= state.prewarm_current();
    changed |= state.poll_vcs_branches();
    changed |= crate::ui::diff_panel::tick(state, tx);
    changed |= state.poll_worktree_bootstraps();
    changed |= state.poll_worktree_setup();
    changed |= state.refresh_local_bindings();
    changed |= state.poll_checkout_provisioning();
    changed |= state.poll_placements();
    changed |= state.poll_question_answers();
    if state.runtime_enabled && state.client.is_none() {
        for index in 0..state.threads.len() {
            if std::mem::take(&mut state.threads[index].native.question_message_connect) {
                state.connect_thread(index);
                changed = true;
            }
        }
    }
    if state.runtime_enabled
        && !state.demo
        && let Ok(server) = crate::mcp::server(state.wake.clone())
    {
        for _ in 0..16 {
            let Some(request) = server.next_request() else {
                break;
            };
            state.mcp_request(request);
            changed = true;
            tx.wake_after(Duration::ZERO);
        }
    }
    if changed {
        state.persistence_scope = None;
    }
    let batch = runtime_poll::poll_threads(state);
    for wake in batch.wakes {
        tx.wake_after(wake);
    }
    changed |= batch.changed;
    for index in batch.connect_indices {
        state.connect_thread(index);
        changed = true;
    }
    // Threads whose provider turn ended or reported a file change: their diff
    // panels are marked stale below (KSG's workspace-mutation refresh bump).
    for index in batch.diff_notes {
        state.diff_note_event(index);
    }
    for (parent, question) in batch.crew_questions {
        state.persistence_scope = None;
        let parent = state.effective_home(parent);
        if let Some(i) = state.threads.iter().position(|t| t.id == parent) {
            state.threads[i].enqueue_injection(question);
            if state.threads[i].pi.is_none() {
                state.connect_thread(i);
            }
        }
    }
    let mut global_changed = false;
    for (thread_id, error) in batch.background_failures {
        if !state.handle_multi_model_async_failure(thread_id, &error) {
            state.notify_background_failure("local", thread_id, error);
        }
        global_changed = true;
    }
    global_changed |= state.pause_offline_errors()
        | state.poll_tasks(Instant::now())
        | state.poll_background_end(tx);
    changed |= state.sync_fresh_session_notice();
    changed |= state.sync_fallback_model(lifecycle::now() as i64);
    if let Some(outcome) = state
        .current_mut()
        .harness_state
        .as_object_mut()
        .and_then(|obj| obj.remove("resetCreditOutcome"))
    {
        changed = true;
        if let Some(err) = outcome["error"].as_str() {
            state.push_toast(
                ToastKind::Error,
                "Could not use the reset credit.",
                Some(err.to_owned()),
                None,
                Some(5_000),
            );
        } else {
            let msg = match outcome["outcome"].as_str() {
                Some("reset") => "Reset applied. Your windows have cleared.",
                Some("nothingToReset") => "Nothing to reset right now.",
                Some("noCredit") => "No reset credit left.",
                Some("alreadyRedeemed") => "That credit was already redeemed.",
                _ => "Could not use the reset credit.",
            };
            let kind = if outcome["outcome"].as_str() == Some("reset") {
                ToastKind::Success
            } else {
                ToastKind::Warning
            };
            state.push_toast(kind, msg, None, None, Some(5_000));
            if state.usage_limits_notice_key.is_some() {
                state.open_usage_limits();
            }
        }
    }
    if global_changed {
        state.persistence_scope = None;
        changed = true;
    }
    if !state.headless
        && let Some(wait) = crate::ui::tick_portraits(state, Instant::now())
    {
        tx.wake_after(wait);
    }
    // A restart click only asks for one. The guard runs after Pi events drained and tasks
    // were delivered, and an approved request stops all further work in this poll.
    if state.restart_for_update {
        state.restart_for_update = false;
        let mut reply = String::new();
        let approval = if batch.event_budget_exhausted {
            reply =
                "Shina is still processing events. Click restart again when it finishes.".into();
            Err("Pending events".to_owned())
        } else {
            state.prepared_exit(true, Some(&mut reply))
        };
        match approval {
            Ok(()) => {
                tx.quit();
                return true;
            }
            Err(_) => {
                state.report_restart_blocker(reply);
                changed = true;
            }
        }
    }
    let at = lifecycle::now();
    let early_wake: Vec<_> = state
        .threads
        .iter()
        .map(|t| {
            t.life.snoozed(at)
                && (state.row_pending(t)
                    || t.life
                        .failed_at
                        .is_some_and(|failed| failed > t.life.snoozed_at.unwrap_or(u64::MAX)))
        })
        .collect();
    let pending: Vec<_> = state.threads.iter().map(|t| state.row_pending(t)).collect();
    for (i, t) in state.threads.iter_mut().enumerate() {
        if t.life
            .snoozed_until
            .is_some_and(|wake| wake > state.last_lifecycle_clock && wake <= at)
        {
            if !state.headless {
                changed = true;
            }
        }
        if t.life.snoozed(at) && early_wake[i] {
            // Store the early edge once. Answering the request must not re-snooze it
            // or make the scheduled wake display a second Woke notification.
            t.life.last_snooze_wake_at = Some(at);
            if let Some(scope) = &mut state.persistence_scope {
                scope.insert(t.id);
            }
            changed = true;
        }
        // KSG's working-shelf grace: a run that just ended keeps its Working place for
        // two seconds so consecutive turns do not bounce the row between shelves.
        let skip =
            pending[i] || t.pinned() || t.life.settled() || t.life.snoozed(at) || t.archived();
        let working = t.status == Status::Running && t.machine == "local";
        let previous = t.working.was_working;
        t.working.observe(at, working, skip);
        changed |= previous != t.working.was_working;
        if let Some(deadline) = t.working.deadline {
            tx.wake_after(Duration::from_millis(deadline.saturating_sub(at)));
        }
        if let Some(wake) = t.life.snoozed_until.filter(|wake| *wake > at) {
            tx.wake_after(Duration::from_millis(wake - at));
        }
        if !state.headless && i == state.active && t.life.woke(at) {
            t.life.last_visited_at = Some(at);
            changed = true;
        }
    }
    state.last_lifecycle_clock = at;
    if batch.run_ended || at.saturating_sub(state.last_sweep) >= 60000 {
        if state.sweep_lifecycle(at) {
            state.persistence_scope = None;
            changed = true;
        }
        // The minute clock also repaints wake countdowns and row ages.
        changed |= !state.headless;
    }
    tx.wake_after(Duration::from_millis(
        60000 - at.saturating_sub(state.last_sweep).min(60000),
    ));
    if let Some(notice) = &state.undo {
        if notice.expires <= at {
            state.undo = None;
            state.deleted_threads.clear();
            state.deleted_records = None;
            changed = true;
        } else {
            tx.wake_after(Duration::from_millis(notice.expires - at));
        }
    }
    changed |= state.expire_toasts();
    for toast in &state.toasts {
        if let Some(expires) = toast.expires_at
            && expires > at
        {
            tx.wake_after(Duration::from_millis(expires - at));
        }
    }
    if let Some((_, expires)) = &state.plan_copied {
        if *expires <= at {
            state.plan_copied = None;
            changed = true;
        } else {
            tx.wake_after(Duration::from_millis(*expires - at));
        }
    }
    if let Some(error) = state
        .store
        .as_ref()
        .and_then(|store| store.errors.try_recv().ok())
    {
        state.storage_error = Some(error);
        changed = true;
    }
    if let Some(error) = state
        .ui_store
        .as_ref()
        .and_then(|store| store.errors.try_recv().ok())
    {
        state.storage_error = Some(error);
        changed = true;
    }
    let (memory_error, memory_healthy) = state.memory_storage_status();
    if let Some(error) = memory_error {
        state.storage_error = Some(error);
        changed = true;
    }
    if state.storage_error.is_some()
        && memory_healthy
        && state
            .store
            .as_ref()
            .is_some_and(crate::store::Store::healthy)
        && state
            .ui_store
            .as_ref()
            .is_some_and(crate::store::Store::healthy)
    {
        state.storage_error = None;
        changed = true;
    }
    changed |= state.poll_approval_sends();
    if state.approval_sends_pending() {
        tx.wake_after(Duration::from_millis(16));
    }
    if changed {
        state.checkpoint();
    }
    changed
}

pub const ZOOM_STEPS: [f32; 10] = [0.67, 0.75, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0];

pub(super) fn next_counter(saved: &Value, maximum: usize) -> Result<usize, String> {
    let minimum = maximum.checked_add(1).ok_or("Invalid id counter.")?;
    let next = if saved.is_null() {
        minimum
    } else {
        usize::try_from(saved.as_u64().ok_or("Invalid id counter.")?)
            .map_err(|_| "Invalid id counter.")?
            .max(minimum)
    };
    if next == usize::MAX {
        return Err("Thread or project ids exhausted.".into());
    }
    Ok(next)
}

pub(crate) fn parse_delegation_target(args: &Value) -> Result<Option<DelegationTarget>, String> {
    let Some(target) = args.get("target") else {
        return Ok(None);
    };
    let object = target.as_object().ok_or(
        "target must be an object with harness or model, and optional provider and options.",
    )?;
    if let Some(key) = object.keys().find(|key| {
        !matches!(
            key.as_str(),
            "machine"
                | "environmentId"
                | "projectId"
                | "repository"
                | "harness"
                | "provider"
                | "model"
                | "options"
        )
    }) {
        return Err(format!(
            "Unknown target setting {key}. Use harness, provider, model and options."
        ));
    }
    let harness = object
        .get("harness")
        .map(|value| {
            value
                .as_str()
                .filter(|id| crate::harness::valid_id(id))
                .map(str::to_owned)
                .ok_or("target.harness must be pi, codex, claude, or an installed ACP id.")
        })
        .transpose()?;
    remote_tasks::target_machine(args)?;
    if let Some(project) = object.get("projectId") {
        if project
            .as_u64()
            .is_none_or(|id| id == 0 || id >= usize::MAX as u64)
        {
            return Err(
                "target.projectId must be a positive project id on the named machine.".into(),
            );
        }
        if remote_tasks::target_machine(args)?.is_none_or(|machine| machine == "local") {
            return Err("target.projectId requires target.environmentId or target.machine.".into());
        }
        if args.get("workspace").is_some() {
            return Err("Pass target.projectId or workspace, not both.".into());
        }
        if object.get("repository").is_some() {
            return Err("Pass target.projectId or target.repository, not both.".into());
        }
    }
    if let Some(repository) = object.get("repository") {
        if repository
            .as_str()
            .is_none_or(|selector| selector.trim().is_empty())
        {
            return Err("target.repository must name a catalog id or repository URL.".into());
        }
        if remote_tasks::target_machine(args)?.is_none_or(|machine| machine == "local") {
            return Err("target.repository requires a linked target.machine.".into());
        }
        if args.get("workspace").is_some() {
            return Err("Pass target.repository or workspace, not both.".into());
        }
    }
    if !object.contains_key("model")
        && harness.is_none()
        && remote_tasks::target_machine(args)?.is_none()
    {
        return Err("target.model is required when target is set without a harness.".into());
    }
    let model = object
        .get("model")
        .map(|value| {
            value.as_str()
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(str::to_owned)
        .ok_or(
            "target.model must be a non-empty exact model id from orchestrator_capabilities.",
        )
        })
        .transpose()?;
    let provider = object
        .get("provider")
        .map(|value| {
            value
                .as_str()
                .filter(|p| !p.is_empty())
                .map(str::to_owned)
                .ok_or("target.provider must be a nonempty string.")
        })
        .transpose()?;
    let reasoning = match object.get("options") {
        None => None,
        Some(options) => {
            let options = options
                .as_object()
                .ok_or("target.options must be an object with reasoning, or its alias thinking.")?;
            let mut reasoning: Option<String> = None;
            for (key, value) in options {
                let level = match key.as_str() {
                    "reasoning" | "thinking" => value
                        .as_str()
                        .map(str::trim)
                        .filter(|level| !level.is_empty())
                        .ok_or_else(|| {
                            format!(
                                "target.options.{key} must be a non-empty reasoning level from orchestrator_capabilities."
                            )
                        })?,
                    _ => {
                        return Err(format!(
                            "Unknown target.options setting {key}. Use reasoning, or its alias thinking."
                        ));
                    }
                };
                if let Some(existing) = &reasoning
                    && existing != level
                {
                    return Err(
                        "target.options.reasoning and target.options.thinking disagree. Pass one level."
                            .into(),
                    );
                }
                reasoning = Some(level.to_owned());
            }
            reasoning
        }
    };
    Ok(Some(DelegationTarget {
        harness,
        provider,
        model,
        reasoning,
    }))
}

pub(super) fn wait_seconds(args: &Value) -> Result<u64, String> {
    match args.get("wait_seconds") {
        None => Ok(0),
        Some(value) => value
            .as_u64()
            .filter(|&seconds| seconds <= 600)
            .ok_or_else(|| "wait_seconds must be an integer from 0 to 600.".into()),
    }
}

/// The closest supported zoom step, so a hand-edited index cannot pick an odd scale.
pub fn snap_zoom(zoom: f32) -> f32 {
    ZOOM_STEPS
        .iter()
        .copied()
        .min_by(|a, b| (a - zoom).abs().total_cmp(&(b - zoom).abs()))
        .unwrap_or(1.0)
}

/// The words the Shina MCP uses for a thread's status.
pub(crate) fn status_word(status: Status) -> &'static str {
    match status {
        Status::Ready => "idle",
        Status::Running => "working",
        Status::Waiting => "needs input",
        Status::Failed => "failed",
        Status::Paused => "paused",
        Status::Completed => "done",
    }
}

pub(super) fn selection_value(thread: &Thread) -> Value {
    thread
        .effective_selection()
        .map_or(Value::Null, |selection| selection.to_value())
}

pub fn string<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}
pub(super) fn content_text(content: &Value) -> String {
    let text = if let Some(text) = content.as_str() {
        text.to_owned()
    } else {
        content
            .as_array()
            .map(|blocks| {
                blocks
                    .iter()
                    .filter(|block| block["type"] == "text")
                    .filter_map(|block| block["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default()
    };
    memory::without_generated(&without_ansi(&text)).into()
}
/// Pi extensions colour their messages for a terminal. Drop the escape sequences (CSI such
/// as `ESC[38;2;144;144;144m`, OSC up to BEL or `ESC\`, and two-byte escapes) so they do not
/// show as stray characters.
pub(super) fn without_ansi(text: &str) -> std::borrow::Cow<'_, str> {
    if !text.contains('\u{1b}') {
        return text.into();
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                for c in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(c) = chars.next() {
                    if c == '\u{7}' {
                        break;
                    }
                    if c == '\u{1b}' {
                        chars.next_if_eq(&'\\');
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out.into()
}
/// Pi keeps user images as typed content blocks. Import each into the local store so the
/// conversation can draw it again without retaining Pi's base64 copy in Shina state.
pub(super) fn content_images(content: &Value) -> Vec<Attachment> {
    content
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter(|block| block["type"] == "image")
                .filter_map(attachments::from_history_part)
                .collect()
        })
        .unwrap_or_default()
}
pub(super) fn tool_detail(args: &Value) -> String {
    ["path", "file_path", "command", "pattern", "query"]
        .iter()
        .find_map(|key| args[key].as_str())
        .map(|s| truncate(s, 140))
        .unwrap_or_default()
}
pub fn truncate(text: &str, count: usize) -> String {
    let mut characters = text.chars();
    let mut result: String = characters.by_ref().take(count).collect();
    if characters.next().is_some() {
        result.push_str("...");
    }
    result
}
