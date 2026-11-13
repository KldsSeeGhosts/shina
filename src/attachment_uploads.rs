//! KSG attachmentUploadQueue.ts, attachmentUploadState.ts and preparingComposerImages.ts.
use crate::{
    attachments::{self, Attachment, Uploaded},
    model::State,
    peer::Link,
    protocol,
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

pub const MAX_UPLOADS_PER_ENVIRONMENT: usize = 3;
pub const UPLOAD_TIMEOUT: Duration = Duration::from_secs(5 * 60);
struct Attempt {
    generation: uuid::Uuid,
    lease: Option<String>,
}
#[derive(Default)]
struct Registry {
    attempts: HashMap<(String, String), Attempt>,
    slots: HashMap<String, Arc<tokio::sync::Semaphore>>,
    denied: HashMap<String, [u8; 32]>,
}
fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(Default::default)
}
fn credential_fingerprint(link: &Link) -> [u8; 32] {
    use sha2::Digest;
    sha2::Sha256::digest(link.token.as_bytes()).into()
}
fn authentication_denied(link: &Link) -> bool {
    let mut registry = registry().lock().expect("attachment upload registry");
    match registry.denied.get(&link.server_id) {
        Some(fingerprint) if *fingerprint == credential_fingerprint(link) => true,
        Some(_) => {
            registry.denied.remove(&link.server_id);
            false
        }
        None => false,
    }
}
fn remember_denial(link: &Link, error: &str) {
    if error.starts_with("Authentication denied.") {
        registry()
            .lock()
            .expect("attachment upload registry")
            .denied
            .insert(link.server_id.clone(), credential_fingerprint(link));
    }
}
fn release_failed(link: &Link, source_id: &str) {
    let attempt = registry()
        .lock()
        .expect("attachment upload registry")
        .attempts
        .remove(&(link.server_id.clone(), source_id.to_owned()));
    if let Some(id) = attempt.and_then(|attempt| attempt.lease) {
        release_later(link.clone(), id);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Capability {
    Unknown,
    Legacy,
    Staged,
}
impl Capability {
    pub fn from_caps(caps: Option<&[Value]>) -> Self {
        match caps {
            None => Self::Unknown,
            Some(caps) if caps.iter().any(|cap| cap == protocol::STAGED_ATTACHMENTS) => {
                Self::Staged
            }
            Some(_) => Self::Legacy,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Phase {
    Preparing,
    Queued,
    Uploading { progress: f32, since: Instant },
    Ready,
    Failed(String),
    Legacy,
}
impl Phase {
    pub fn blocks_send(&self) -> bool {
        matches!(
            self,
            Self::Preparing | Self::Queued | Self::Uploading { .. }
        )
    }
    pub fn label(&self, elapsed: Duration) -> Option<String> {
        match self {
            Self::Preparing => Some("Preparing".into()),
            Self::Uploading { progress, .. } if elapsed >= Duration::from_millis(150) => {
                Some(progress_label(*progress))
            }
            Self::Failed(_) => Some("upload failed".into()),
            _ => None,
        }
    }
}
pub fn progress_label(progress: f32) -> String {
    format!(
        "{}%",
        (if progress.is_finite() {
            progress.clamp(0.0, 1.0)
        } else {
            0.0
        } * 100.0)
            .floor() as u32
    )
}

enum Input {
    Path(PathBuf),
    Image(String, Vec<u8>),
}
enum Event {
    Thumbnail(Vec<u8>),
    Prepared(Result<Attachment, String>),
    Progress(f32),
    Uploaded(Result<Uploaded, String>),
}

pub struct Entry {
    pub machine: String,
    pub thread: usize,
    pub id: String,
    pub name: String,
    pub image: bool,
    pub phase: Phase,
    input: Option<Input>,
    worker: Option<mpsc::Receiver<Event>>,
    cancel: Arc<AtomicBool>,
    image_chip: bool,
    link: Option<Link>,
    verifying: bool,
    uploaded: Option<Uploaded>,
    previous: Option<(Link, Uploaded)>,
    thumbnail_bytes: Option<Arc<[u8]>>,
    thumbnail_handle: std::cell::RefCell<Option<rune::Image>>,
}

impl Entry {
    pub fn thumbnail(&self, cache: &rune::ImageCache) -> Option<rune::Image> {
        if let Some(image) = self.thumbnail_handle.borrow().as_ref() {
            return Some(image.clone());
        }
        let image = cache.load(self.thumbnail_bytes.clone()?);
        *self.thumbnail_handle.borrow_mut() = Some(image.clone());
        Some(image)
    }
}
#[cfg(test)]
impl Entry {
    pub fn fixture(machine: &str, thread: usize, attachment: &Attachment, phase: Phase) -> Self {
        Self {
            machine: machine.into(),
            thread,
            id: attachment.id.clone(),
            name: attachment.name.clone(),
            image: attachment.is_image(),
            phase,
            input: None,
            worker: None,
            cancel: Arc::new(AtomicBool::new(false)),
            image_chip: false,
            link: None,
            verifying: false,
            uploaded: None,
            previous: None,
            thumbnail_bytes: None,
            thumbnail_handle: Default::default(),
        }
    }
}

#[derive(Default)]
pub struct Queue {
    pub entries: Vec<Entry>,
    connected: HashMap<String, bool>,
}

fn target_mut<'a>(
    state: &'a mut State,
    machine: &str,
    id: usize,
) -> Option<&'a mut crate::model::Thread> {
    if machine == state.machines.selected {
        state.threads.iter_mut().find(|thread| thread.id == id)
    } else {
        state
            .machines
            .threads
            .get_mut(machine)?
            .iter_mut()
            .find(|thread| thread.id == id)
    }
}

fn connection(state: &State, machine: &str) -> (Capability, bool, Option<Link>) {
    let client = state.client.as_ref().and_then(|client| {
        if machine == "local" {
            Some(client)
        } else {
            client.linked.get(machine)
        }
    });
    let Some(client) = client else {
        return (Capability::Legacy, true, None);
    };
    let info = client.info.lock().expect("connection info");
    let capability = Capability::from_caps(info["capabilities"].as_array().map(Vec::as_slice));
    let link = client.remote.clone().or_else(|| {
        let discovery = crate::server_auth::Discovery::read(&state.data_dir).ok()?;
        let saved = crate::store::load(&state.data_dir.join("client-credential.json")).ok()??;
        if saved["serverId"] != discovery.server_id {
            return None;
        }
        Some(Link {
            server_id: discovery.server_id,
            host: format!("127.0.0.1:{}", discovery.port),
            device_id: saved["deviceId"].as_str()?.into(),
            token: saved["token"].as_str()?.into(),
        })
    });
    (capability, client.commands.connected(), link)
}

impl Queue {
    pub fn preparing_count(&self, machine: &str, thread: usize) -> usize {
        self.entries
            .iter()
            .filter(|entry| {
                entry.machine == machine
                    && entry.thread == thread
                    && entry.image
                    && matches!(entry.phase, Phase::Preparing)
            })
            .count()
    }
    pub fn block_reason(&self, machine: &str, thread: usize) -> Option<&'static str> {
        if self.entries.iter().any(|entry| entry.machine == machine && entry.thread == thread
            && matches!(&entry.phase, Phase::Failed(reason) if reason.starts_with("Authentication denied."))) {
            return Some("Authentication denied. Reconnect this linked machine manually before transferring files.");
        }
        let count = self
            .entries
            .iter()
            .filter(|entry| {
                entry.machine == machine && entry.thread == thread && entry.phase.blocks_send()
            })
            .count();
        match count {
            0 => None,
            1 => Some("Attachment still uploading"),
            _ => Some("Attachments still uploading"),
        }
    }
    pub fn phase(&self, machine: &str, thread: usize, id: &str) -> Option<&Phase> {
        self.entries
            .iter()
            .find(|entry| entry.machine == machine && entry.thread == thread && entry.id == id)
            .map(|entry| &entry.phase)
    }
    fn enqueue(&mut self, state: &mut State, index: usize, input: Input, skip_image_chip: bool) {
        let machine = state.machines.selected.clone();
        let thread_id = state.threads[index].id;
        self.enqueue_on(state, machine, thread_id, input, skip_image_chip);
    }
    fn enqueue_on(
        &mut self,
        state: &mut State,
        machine: String,
        thread_id: usize,
        input: Input,
        skip_image_chip: bool,
    ) {
        let (name, mime, bytes) = match &input {
            Input::Path(path) => {
                let name = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                let metadata = match std::fs::metadata(path) {
                    Ok(metadata) => metadata,
                    Err(error) => {
                        if let Some(target) = target_mut(state, &machine, thread_id) {
                            target.attachment_error =
                                Some(format!("Couldn't get the path of \"{name}\"\n{error}"));
                        }
                        return;
                    }
                };
                let mime = crate::attachment_intake::native_mime(&name).to_owned();
                (name, mime, metadata.len())
            }
            Input::Image(name, bytes) => (
                name.clone(),
                crate::attachment_intake::native_mime(name).into(),
                bytes.len() as u64,
            ),
        };
        let kind = crate::attachment_intake::classify(&name, &mime);
        let image = kind == crate::attachment_intake::Kind::Image;
        let files_supported = state.file_attachment_staging_supported_on(&machine);
        let file_maximum = state
            .file_attachment_staging_limit_on(&machine)
            .unwrap_or(attachments::MAX_FILE_BYTES);
        let Some(target) = target_mut(state, &machine, thread_id) else {
            return;
        };
        let replacement = (!image)
            .then(|| {
                target.draft_images.iter().position(|a| {
                    a.matches_reattach(
                        &name,
                        &crate::attachment_intake::file_mime(&name, &mime),
                        bytes,
                    )
                })
            })
            .flatten();
        let error = if replacement.is_none()
            && target.draft_images.len() + self.preparing_count(&machine, thread_id)
                >= attachments::MAX_ATTACHMENTS
        {
            Some(format!(
                "You can attach up to {} files per message.",
                attachments::MAX_ATTACHMENTS
            ))
        } else if kind == crate::attachment_intake::Kind::UnsupportedImage {
            Some(format!(
                "'{name}' is not a supported image type. Attach GIF, HEIC, HEIF, JPEG, PNG, or WebP images."
            ))
        } else if !image && !files_supported {
            Some("This server does not support file attachments.".into())
        } else if !image && bytes == 0 {
            Some(format!("'{name}' is empty or could not be read."))
        } else if !image && bytes > file_maximum {
            Some(protocol::file_attachment_too_large(&name, file_maximum))
        } else {
            None
        };
        if let Some(error) = error {
            target.attachment_error = Some(error);
            return;
        }
        use sha2::Digest;
        let id = if let Some(index) = replacement {
            target.draft_images[index].id.clone()
        } else if image {
            format!("{}.png", uuid::Uuid::new_v4())
        } else {
            format!(
                "file-{:x}.bin",
                sha2::Sha256::digest(uuid::Uuid::new_v4().as_bytes())
            )
        };
        let image_chip = !skip_image_chip
            && (target.draft.anchor != target.draft.focus
                || !crate::composer_context::strip_inline_context_references(
                    &crate::composer_rich::markdown(&target.draft),
                )
                .trim()
                .is_empty());
        if !image {
            let path = match &input {
                Input::Path(path) => Some(path.clone()),
                _ => None,
            };
            let mut placeholder = Attachment {
                id: id.clone(),
                name: name.clone(),
                mime: crate::attachment_intake::file_mime(&name, &mime),
                width: 0,
                height: 0,
                bytes,
                path,
                asset_id: None,
                uploaded: None,
                source: None,
            };
            if let Some(index) = replacement {
                // A restored path-reference marker has an older logical id. Preserve
                // its new file's path identity until the worker publishes an owned blob.
                if let Input::Path(path) = &input
                    && let Ok(source) = attachments::reference_file(path)
                {
                    placeholder.asset_id = Some(source.id);
                }
                target.draft_images[index] = placeholder;
                self.remove(&machine, thread_id, &id);
            } else {
                target.draft_images.push(placeholder.clone());
                target.insert_attachment_chips(&[placeholder], false);
            }
        }
        self.entries.push(Entry {
            machine,
            thread: thread_id,
            id,
            name,
            image,
            phase: Phase::Preparing,
            input: Some(input),
            worker: None,
            cancel: Arc::new(AtomicBool::new(false)),
            image_chip,
            link: None,
            verifying: false,
            uploaded: None,
            previous: None,
            thumbnail_bytes: None,
            thumbnail_handle: Default::default(),
        });
    }

    pub fn remove(&mut self, machine: &str, thread: usize, id: &str) {
        self.entries.retain(|entry| {
            let keep = entry.machine != machine || entry.thread != thread || entry.id != id;
            if !keep {
                entry.cancel.store(true, Ordering::Release);
                if let Some(link) = &entry.link {
                    release_failed(link, &entry.id);
                }
                if let (Some(link), Some(uploaded)) = (&entry.link, &entry.uploaded) {
                    release_later(link.clone(), uploaded.lease_id.clone());
                }
                if let Some((link, uploaded)) = &entry.previous {
                    release_later(link.clone(), uploaded.lease_id.clone());
                }
            }
            keep
        });
    }
    pub fn retry(&mut self, machine: &str, thread: usize, id: &str) {
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| entry.machine == machine && entry.thread == thread && entry.id == id)
        {
            if matches!(&entry.phase, Phase::Failed(reason) if reason.starts_with("Authentication denied."))
            {
                return;
            }
            entry.cancel.store(true, Ordering::Release);
            if let Some(link) = &entry.link {
                release_failed(link, &entry.id);
            }
            entry.cancel = Arc::new(AtomicBool::new(false));
            entry.worker = None;
            entry.phase = if entry.input.is_some() {
                Phase::Preparing
            } else {
                Phase::Queued
            };
            entry.verifying = false;
        }
    }

    pub fn poll(&mut self, state: &mut State) -> bool {
        let mut changed = false;
        let mut dirty = std::collections::HashSet::new();
        let active = (state.machines.selected.clone(), state.current().id);
        let mut refocus = false;
        // Resume persisted drafts, including inactive drafts. Logical ids survive moves.
        let mut drafts: Vec<_> = state
            .threads
            .iter()
            .map(|t| {
                (
                    state.machines.selected.clone(),
                    t.id,
                    t.draft_images.clone(),
                )
            })
            .collect();
        for (machine, threads) in &state.machines.threads {
            if machine != &state.machines.selected {
                drafts.extend(
                    threads
                        .iter()
                        .map(|t| (machine.clone(), t.id, t.draft_images.clone())),
                );
            }
        }
        for (machine, thread, attachments) in drafts {
            for attachment in attachments {
                if attachment.mime == "inode/directory" || attachment.needs_reattach() {
                    continue;
                }
                if let Some(entry) = self
                    .entries
                    .iter_mut()
                    .find(|entry| entry.id == attachment.id)
                {
                    if entry.machine != machine {
                        entry.cancel.store(true, Ordering::Release);
                        entry.cancel = Arc::new(AtomicBool::new(false));
                        entry.worker = None;
                        if let (Some(link), Some(uploaded)) =
                            (entry.link.take(), entry.uploaded.take())
                        {
                            entry.previous = Some((link, uploaded));
                        }
                        entry.phase = Phase::Queued;
                        entry.verifying = true;
                    }
                    entry.machine = machine.clone();
                    entry.thread = thread;
                    continue;
                }
                let uploaded = attachment.uploaded.clone();
                self.entries.push(Entry {
                    machine: machine.clone(),
                    thread,
                    id: attachment.id,
                    name: attachment.name,
                    image: attachment.path.is_none(),
                    phase: Phase::Queued,
                    input: None,
                    worker: None,
                    cancel: Arc::new(AtomicBool::new(false)),
                    image_chip: false,
                    link: None,
                    verifying: uploaded.is_some(),
                    uploaded,
                    previous: None,
                    thumbnail_bytes: None,
                    thumbnail_handle: Default::default(),
                });
            }
        }
        let mut active_uploads = HashMap::<String, usize>::new();
        let mut preparing = self
            .entries
            .iter()
            .any(|entry| matches!(entry.phase, Phase::Preparing) && entry.worker.is_some());
        for entry in &self.entries {
            if entry.worker.is_some() && !matches!(entry.phase, Phase::Preparing) {
                *active_uploads.entry(entry.machine.clone()).or_default() += 1;
            }
        }
        let mut keep = Vec::new();
        let mut connections = HashMap::new();
        for mut entry in std::mem::take(&mut self.entries) {
            let file_maximum = state.file_attachment_staging_limit_on(&entry.machine);
            let (capability, connected, link) = connections
                .entry(entry.machine.clone())
                .or_insert_with(|| connection(state, &entry.machine))
                .clone();
            let denied = link.as_ref().is_some_and(authentication_denied);
            if matches!(&entry.phase, Phase::Failed(error) if error.starts_with("Authentication denied."))
                && !denied
                && link
                    .as_ref()
                    .zip(entry.link.as_ref())
                    .is_some_and(|(current, previous)| current.token != previous.token)
            {
                entry.phase = Phase::Queued;
                entry.cancel = Arc::new(AtomicBool::new(false));
                entry.worker = None;
                entry.verifying = entry.uploaded.is_some();
                changed = true;
            }
            if denied && matches!(entry.phase, Phase::Queued | Phase::Uploading { .. }) {
                entry.cancel.store(true, Ordering::Release);
                entry.worker = None;
                entry.link = link.clone();
                entry.phase = Phase::Failed("Authentication denied. Reconnect this linked machine manually before transferring files.".into());
                changed = true;
            }
            let reconnected = connected && self.connected.get(&entry.machine) == Some(&false);
            let Some(target) = target_mut(state, &entry.machine, entry.thread) else {
                entry.cancel.store(true, Ordering::Release);
                continue;
            };
            if !matches!(entry.phase, Phase::Preparing)
                && !target.draft_images.iter().any(|a| a.id == entry.id)
            {
                if target.remote_send.is_some() {
                    keep.push(entry);
                    continue;
                }
                entry.cancel.store(true, Ordering::Release);
                continue;
            }
            if let Some(receiver) = entry.worker.take() {
                let mut finished = false;
                loop {
                    let event = match receiver.try_recv() {
                        Ok(event) => event,
                        Err(mpsc::TryRecvError::Empty) => break,
                        Err(mpsc::TryRecvError::Disconnected) => {
                            if !finished {
                                entry.phase = Phase::Failed("Upload failed".into());
                                changed = true;
                            }
                            finished = true;
                            break;
                        }
                    };
                    changed = true;
                    if matches!(event, Event::Prepared(_) | Event::Uploaded(_)) {
                        finished = true;
                    }
                    match event {
                        Event::Thumbnail(bytes) => {
                            entry.thumbnail_bytes = Some(bytes.into());
                            *entry.thumbnail_handle.borrow_mut() = None;
                            changed = true;
                        }
                        Event::Progress(progress) => {
                            if let Phase::Uploading {
                                progress: current, ..
                            } = &mut entry.phase
                            {
                                *current = progress;
                            }
                        }
                        Event::Prepared(result) => match result {
                            Ok(mut attachment) => {
                                dirty.insert((entry.machine.clone(), entry.thread));
                                refocus |= active == (entry.machine.clone(), entry.thread);
                                attachment.asset_id = Some(attachment.id.clone());
                                attachment.id = entry.id.clone();
                                entry.name = attachment.name.clone();
                                entry.input = None;
                                for record in &mut target.draft_context {
                                    if let crate::composer_context::ComposerContextData::File {
                                        attachment_id,
                                        name,
                                        mime_type,
                                        size_bytes,
                                    } = &mut record.data
                                        && attachment_id == &entry.id
                                    {
                                        *name = attachment.name.clone();
                                        *mime_type = attachment.mime.clone();
                                        *size_bytes = attachment.bytes;
                                    }
                                }
                                if let Some(existing) =
                                    target.draft_images.iter_mut().find(|a| a.id == entry.id)
                                {
                                    *existing = attachment.clone();
                                } else {
                                    target.draft_images.push(attachment.clone());
                                    target.insert_prepared_image_chips(
                                        &[attachment],
                                        entry.image_chip,
                                    );
                                }
                                entry.phase = Phase::Queued;
                            }
                            Err(error) => {
                                if entry.image {
                                    target.draft_images.retain(|a| a.id != entry.id);
                                }
                                target.attachment_error = Some(error.clone());
                                entry.phase = Phase::Failed(error);
                            }
                        },
                        Event::Uploaded(result) => match result {
                            Ok(uploaded) => {
                                dirty.insert((entry.machine.clone(), entry.thread));
                                if let Some(attachment) =
                                    target.draft_images.iter_mut().find(|a| a.id == entry.id)
                                {
                                    attachment.uploaded = Some(uploaded.clone());
                                }
                                entry.uploaded = Some(uploaded);
                                if let Some((link, previous)) = entry.previous.take() {
                                    release_later(link, previous.lease_id);
                                }
                                entry.phase = Phase::Ready;
                            }
                            Err(error) => {
                                if error == "Uploaded file expired. Remove it and attach it again."
                                    && let Some(attachment) =
                                        target.draft_images.iter_mut().find(|a| a.id == entry.id)
                                    && !attachment.is_image()
                                    && !attachments::path_for(attachment)
                                        .is_some_and(|p| p.is_file())
                                {
                                    attachment.uploaded = None;
                                    entry.uploaded = None;
                                    dirty.insert((entry.machine.clone(), entry.thread));
                                }
                                entry.phase = Phase::Failed(error);
                            }
                        },
                    }
                }
                if !finished {
                    entry.worker = Some(receiver);
                }
            }
            if !entry.image
                && let Some(file) = target.draft_images.iter().find(|a| a.id == entry.id)
                && let Some(reason) = match file_maximum {
                    Some(maximum) if file.bytes > maximum =>
                        Some(protocol::file_attachment_too_large(&file.name, maximum)),
                    None => Some("This server does not accept file attachments right now. Remove the files to send.".into()),
                    _ => None,
                }
            {
                entry.cancel.store(true, Ordering::Release);
                entry.worker = None;
                if entry.uploaded.is_none() && let Some(link) = &entry.link {
                    release_failed(link, &entry.id);
                }
                changed |= !matches!(&entry.phase, Phase::Failed(current) if current == &reason);
                entry.phase = Phase::Failed(reason);
                keep.push(entry);
                continue;
            }
            // A capability change cancels the transfer. Persisted file stamps remain owned
            // by their draft; images and uncommitted transfers fall back to legacy sending.
            if capability == Capability::Legacy
                && !matches!(entry.phase, Phase::Preparing | Phase::Legacy)
            {
                entry.cancel.store(true, Ordering::Release);
                entry.worker = None;
                if let Some(link) = &entry.link {
                    release_failed(link, &entry.id);
                }
                if entry.image {
                    if let (Some(link), Some(uploaded)) = (&entry.link, entry.uploaded.take()) {
                        release_later(link.clone(), uploaded.lease_id);
                    }
                    if let Some(attachment) =
                        target.draft_images.iter_mut().find(|a| a.id == entry.id)
                    {
                        attachment.uploaded = None;
                    }
                }
                entry.phase = Phase::Legacy;
                changed = true;
            } else if capability == Capability::Staged && matches!(entry.phase, Phase::Legacy) {
                entry.cancel = Arc::new(AtomicBool::new(false));
                entry.phase = Phase::Queued;
            }
            if let Some((previous_link, uploaded)) = &entry.previous
                && link
                    .as_ref()
                    .is_some_and(|link| link.server_id == previous_link.server_id)
            {
                if let Some(attachment) = target.draft_images.iter_mut().find(|a| a.id == entry.id)
                {
                    attachment.uploaded = Some(uploaded.clone());
                }
                entry.uploaded = Some(uploaded.clone());
                entry.link = Some(previous_link.clone());
                entry.previous = None;
                entry.phase = Phase::Ready;
            }
            if reconnected
                && matches!(&entry.phase, Phase::Failed(error) if !error.starts_with("Authentication denied."))
            {
                entry.cancel = Arc::new(AtomicBool::new(false));
                entry.phase = Phase::Queued;
                entry.worker = None;
            }
            if entry.worker.is_none() && matches!(entry.phase, Phase::Preparing) && !preparing {
                if let Some(input) = entry.input.take() {
                    if let Input::Path(path) = &input {
                        entry.input = Some(Input::Path(path.clone()));
                    }
                    preparing = true;
                    let (send, receive) = mpsc::channel();
                    let wake = state.wake.clone();
                    let profile = state.data_dir.clone();
                    let cancel = entry.cancel.clone();
                    let image = entry.image;
                    std::thread::spawn(move || {
                        let thumbnail = |bytes| {
                            if !cancel.load(Ordering::Acquire) {
                                let _ = send.send(Event::Thumbnail(bytes));
                                wake();
                            }
                        };
                        let prepare = || match input {
                            Input::Path(path) => {
                                if image {
                                    attachments::attach_image_file_with_thumbnail(&path, thumbnail)
                                } else {
                                    attachments::attach_file(&path).and_then(|attachment| {
                                        if attachment.is_image() {
                                            Ok(attachment)
                                        } else {
                                            attachments::stage_transfer_file(&attachment)
                                        }
                                    })
                                }
                            }
                            Input::Image(name, bytes) => {
                                crate::attachment_intake::prepare_with_thumbnail(
                                    &name,
                                    crate::attachment_intake::native_mime(&name),
                                    bytes,
                                    attachments::MAX_IMAGE_BYTES as usize,
                                    thumbnail,
                                )
                                .map_err(|failure| failure.message(&name))
                                .and_then(|(name, bytes)| attachments::attach_bytes(&name, bytes))
                            }
                        };
                        #[cfg(test)]
                        let result = crate::store::with_directory(profile, prepare);
                        #[cfg(not(test))]
                        let result = {
                            let _ = profile;
                            prepare()
                        };
                        if !cancel.load(Ordering::Acquire) {
                            let _ = send.send(Event::Prepared(result));
                        }
                        wake();
                    });
                    entry.worker = Some(receive);
                }
            } else if entry.worker.is_none()
                && matches!(entry.phase, Phase::Queued)
                && capability == Capability::Staged
                && connected
                && *active_uploads.get(&entry.machine).unwrap_or(&0) < MAX_UPLOADS_PER_ENVIRONMENT
            {
                if let Some(link) = link {
                    let attachment = target
                        .draft_images
                        .iter()
                        .find(|a| a.id == entry.id)
                        .cloned();
                    if let Some(attachment) = attachment {
                        *active_uploads.entry(entry.machine.clone()).or_default() += 1;
                        let (send, receive) = mpsc::channel();
                        let cancel = entry.cancel.clone();
                        let wake = state.wake.clone();
                        let verify = entry.verifying;
                        let upload_link = link.clone();
                        #[cfg(test)]
                        let test_directory = state.data_dir.clone();
                        std::thread::spawn(move || {
                            let work = || {
                                tokio::runtime::Builder::new_current_thread()
                                    .enable_all()
                                    .build()
                                    .map_err(|e| e.to_string())
                                    .and_then(|runtime| {
                                        runtime.block_on(async {
                                            if verify && let Some(uploaded) = &attachment.uploaded
                                                && uploaded.environment_id == upload_link.server_id {
                                                if verify_upload(&upload_link, uploaded).await? {
                                                    return Ok(uploaded.clone());
                                                }
                                                if !attachment.is_image()
                                                    && !attachments::path_for(&attachment).is_some_and(|p| p.is_file()) {
                                                    return Err("Uploaded file expired. Remove it and attach it again.".into());
                                                }
                                            }
                                            upload(&upload_link, &attachment, &cancel, |progress| {
                                                let _ = send.send(Event::Progress(progress));
                                                wake();
                                            })
                                            .await
                                        })
                                    })
                            };
                            #[cfg(test)]
                            let result = crate::store::with_directory(test_directory, work);
                            #[cfg(not(test))]
                            let result = work();
                            let _ = send.send(Event::Uploaded(result));
                            wake();
                        });
                        entry.link = Some(link);
                        entry.worker = Some(receive);
                        entry.verifying = false;
                        entry.phase = Phase::Uploading {
                            progress: 0.0,
                            since: Instant::now(),
                        };
                        changed = true;
                    }
                }
            }
            keep.push(entry);
        }
        for entry in &keep {
            if let Some((_, connected, _)) = connections.get(&entry.machine) {
                self.connected.insert(entry.machine.clone(), *connected);
            }
        }
        self.entries = keep;
        for (machine, thread) in dirty {
            persist_cached_draft(state, &machine, thread);
        }
        if refocus {
            state.restore_composer_focus(crate::ui::composer_ui::FocusReason::AttachmentInserted);
        }
        changed
    }
}

impl Drop for Queue {
    fn drop(&mut self) {
        for entry in &self.entries {
            entry.cancel.store(true, Ordering::Release);
        }
    }
}

impl State {
    pub(crate) fn pasted_text_staging_supported(&self) -> bool {
        if matches!(self.current().harness.as_str(), "pi" | "codex" | "claude") {
            return true;
        }
        let client = self.client.as_ref().and_then(|client| {
            if self.machines.selected == "local" {
                Some(client)
            } else {
                client.linked.get(&self.machines.selected)
            }
        });
        client.is_none_or(|client| {
            client.info.lock().expect("connection info")["capabilities"]
                .as_array()
                .is_none_or(|caps| {
                    caps.iter()
                        .any(|cap| cap == protocol::PASTED_TEXT_ATTACHMENTS)
                })
        })
    }
    pub(crate) fn attachment_preview_link(&self) -> Option<Link> {
        connection(self, &self.machines.selected).2
    }
    pub(crate) fn file_attachment_staging_supported_on(&self, machine: &str) -> bool {
        self.file_attachment_staging_limit_on(machine).is_some()
    }
    pub(crate) fn file_attachment_staging_limit_on(&self, machine: &str) -> Option<u64> {
        let client = self.client.as_ref().and_then(|client| {
            if machine == "local" {
                Some(client)
            } else {
                client.linked.get(machine)
            }
        });
        let Some(client) = client else {
            return Some(attachments::MAX_FILE_BYTES);
        };
        let info = client.info.lock().expect("connection info");
        protocol::file_attachment_limit(&info)
    }
    pub(crate) fn set_attachment_target_error(
        &mut self,
        machine: &str,
        thread: usize,
        error: String,
    ) {
        if let Some(target) = target_mut(self, machine, thread) {
            target.attachment_error = Some(error);
        }
    }
    pub(crate) fn attach_paths_on_to(&mut self, machine: &str, thread: usize, paths: &[PathBuf]) {
        if machine == self.machines.selected {
            self.attach_paths_to(thread, paths);
            return;
        }
        let local = machine == "local"
            || self
                .client
                .as_ref()
                .and_then(|client| client.linked.get(machine))
                .and_then(|client| client.remote.as_ref())
                .is_some_and(|link| {
                    url::Url::parse(&format!("http://{}", link.host))
                        .ok()
                        .and_then(|url| url.host_str().map(str::to_owned))
                        .is_some_and(|host| {
                            host == "localhost"
                                || host
                                    .parse::<std::net::IpAddr>()
                                    .is_ok_and(|ip| ip.is_loopback())
                        })
                });
        let Some(target) = target_mut(self, machine, thread) else {
            return;
        };
        if target.archived()
            || target.request.is_some()
            || target.native.operating()
            || target.configuring
        {
            return;
        }
        target.attachment_error = None;
        let mut queue = std::mem::take(&mut self.attachment_uploads);
        for path in paths {
            if path.is_dir() {
                if !local {
                    self.set_attachment_target_error(
                        machine,
                        thread,
                        "Folders can only be referenced on a server running on this computer"
                            .into(),
                    );
                    continue;
                }
                match path.canonicalize() {
                    Ok(path) => {
                        if let Some(target) = target_mut(self, machine, thread) {
                            let label = path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into_owned();
                            let token = crate::composer_tokens::Token::File {
                                path,
                                directory: true,
                                source: None,
                            };
                            let end = target.draft.text.len();
                            let boundary = target
                                .draft
                                .text
                                .chars()
                                .next_back()
                                .is_some_and(|c| !c.is_whitespace());
                            target.draft.undo_group(|edit| {
                                edit.set_selection(end, end);
                                if boundary {
                                    edit.insert(" ");
                                }
                                let start = edit.focus;
                                edit.insert(&label);
                                edit.replace_with_pill(start..edit.focus, token.id(), &label);
                                edit.insert(" ");
                            });
                        }
                    }
                    Err(_) => self.set_attachment_target_error(
                        machine,
                        thread,
                        format!(
                            "Couldn't get the path of \"{}\"\nType the folder path with @ instead.",
                            path.file_name().unwrap_or_default().to_string_lossy()
                        ),
                    ),
                }
            } else {
                queue.enqueue_on(
                    self,
                    machine.into(),
                    thread,
                    Input::Path(path.clone()),
                    false,
                );
            }
        }
        self.attachment_uploads = queue;
        persist_cached_draft(self, machine, thread);
        self.checkpoint();
    }
    pub fn attachment_send_block_reason(&self) -> Option<String> {
        if !self.pasted_text_staging_supported()
            && self
                .current()
                .draft_images
                .iter()
                .any(|a| a.source == Some(attachments::AttachmentSource::PastedText))
        {
            return Some(protocol::PASTED_TEXT_UPDATE_REQUIRED.into());
        }
        if self
            .current()
            .draft_images
            .iter()
            .any(Attachment::needs_reattach)
        {
            return Some("Attach missing files again or remove them to send.".into());
        }
        if let Some(reason) = self
            .attachment_uploads
            .block_reason(&self.machines.selected, self.current().id)
        {
            return Some(reason.into());
        }
        if !self.current().draft_images.is_empty()
            && self.attachment_uploads.entries.iter().any(|entry| {
                entry.machine == self.machines.selected
                    && entry.link.as_ref().is_some_and(authentication_denied)
            })
        {
            return Some("Authentication denied. Reconnect this linked machine manually before transferring files.".into());
        }
        let has_files = self
            .current()
            .draft_images
            .iter()
            .any(|a| !a.is_image() && a.mime != "inode/directory");
        let client = self.client.as_ref().and_then(|client| {
            if self.machines.selected == "local" {
                Some(client)
            } else {
                client.linked.get(&self.machines.selected)
            }
        });
        let client = client?;
        let info = client.info.lock().expect("connection info");
        if info["capabilities"]
            .as_array()
            .is_some_and(|caps| !caps.iter().any(|cap| cap == protocol::STAGED_ATTACHMENTS))
            && self.current().draft_images.iter().any(|a| {
                !a.is_image()
                    && a.uploaded.is_some()
                    && !attachments::path_for(a).is_some_and(|p| p.is_file())
            })
        {
            return Some("This server does not support staged attachments. Update it, or remove and attach these files again to send.".into());
        }
        if info["capabilities"]
            .as_array()
            .is_some_and(|caps| !caps.iter().any(|cap| cap == protocol::ATTACHMENT_LIMITS))
        {
            let images: Vec<_> = self
                .current()
                .draft_images
                .iter()
                .filter(|a| a.is_image())
                .collect();
            let bytes = images
                .iter()
                .map(|image| image.bytes)
                .fold(0u64, u64::saturating_add);
            if let Some(reason) = attachments::legacy_peer_limit_error(images.len(), bytes) {
                return Some(reason);
            }
        }
        if !has_files {
            return None;
        }
        match info["capabilities"].as_array() {
            None => Some("Waiting for the server before file attachments can send".into()),
            Some(caps) if !caps.iter().any(|cap| cap == protocol::FILE_ATTACHMENTS) => Some(
                "This server does not accept file attachments right now. Remove the files to send."
                    .into(),
            ),
            _ => match protocol::file_attachment_limit(&info) {
                None => Some("This server does not accept file attachments right now. Remove the files to send.".into()),
                Some(maximum) => self.current().draft_images.iter()
                    .find(|file| !file.is_image() && file.mime != "inode/directory" && file.bytes > maximum)
                    .map(|file| protocol::file_attachment_too_large(&file.name, maximum)),
            },
        }
    }
    pub(crate) fn enqueue_attachment_path(&mut self, index: usize, path: PathBuf) {
        let mut queue = std::mem::take(&mut self.attachment_uploads);
        queue.enqueue(self, index, Input::Path(path), false);
        self.attachment_uploads = queue;
    }
    pub(crate) fn enqueue_attachment_image(
        &mut self,
        index: usize,
        name: String,
        bytes: Vec<u8>,
        skip_chip: bool,
    ) {
        let mut queue = std::mem::take(&mut self.attachment_uploads);
        queue.enqueue(self, index, Input::Image(name, bytes), skip_chip);
        self.attachment_uploads = queue;
    }
    pub(crate) fn poll_attachment_uploads(&mut self) -> bool {
        if self.background.handle.is_none() {
            return false;
        }
        let mut queue = std::mem::take(&mut self.attachment_uploads);
        let changed = queue.poll(self);
        self.attachment_uploads = queue;
        if self
            .attachment_uploads
            .block_reason(&self.machines.selected, self.current().id)
            .is_none()
            && matches!(
                self.current().attachment_error.as_deref(),
                Some("Attachment still uploading" | "Attachments still uploading")
            )
        {
            self.current_mut().attachment_error = None;
        }
        if changed {
            self.checkpoint();
        }
        changed
    }
    pub fn retry_attachment_upload(&mut self, id: &str) {
        let thread = self.current().id;
        self.attachment_uploads
            .retry(&self.machines.selected, thread, id);
        (self.wake)();
    }
}

fn persist_cached_draft(state: &mut State, machine: &str, thread: usize) {
    if machine == state.machines.selected {
        return;
    }
    let Some(target) = state
        .machines
        .threads
        .get(machine)
        .and_then(|threads| threads.iter().find(|target| target.id == thread))
    else {
        return;
    };
    if target.queued_edit.is_some() {
        return;
    }
    let draft = &target.draft;
    let images = &target.draft_images;
    let context = &target.draft_context;
    let value = if images.is_empty()
        && context.is_empty()
        && !crate::composer_rich::has_format(draft)
        && draft.pills().next().is_none()
    {
        json!(draft.text)
    } else {
        json!({"text":draft.text,"richHtml":draft.to_html(),
        "images":images.iter().map(Attachment::to_value).collect::<Vec<_>>(),
        "context":if context.is_empty() { Value::Null } else { json!({"version":1,"records":context}) }})
    };
    if state.machines.ui[machine].is_null() {
        state.machines.ui[machine] = json!({});
    }
    if state.machines.ui[machine]["drafts"].is_null() {
        state.machines.ui[machine]["drafts"] = json!({});
    }
    state.machines.ui[machine]["drafts"][thread.to_string()] = value;
}

fn http() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(UPLOAD_TIMEOUT)
        .build()
        .map_err(|_| "Upload could not start".into())
}
async fn response(response: reqwest::Response) -> Result<reqwest::Response, String> {
    if matches!(response.status().as_u16(), 401 | 403) {
        return Err("Authentication denied. Reconnect this linked machine manually before transferring files.".into());
    }
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status().as_u16();
    Err(response
        .json::<Value>()
        .await
        .ok()
        .and_then(|value| value["error"]["message"].as_str().map(str::to_owned))
        .unwrap_or_else(|| format!("Upload rejected ({status})")))
}

pub async fn verify_upload(link: &Link, uploaded: &Uploaded) -> Result<bool, String> {
    if authentication_denied(link) {
        return Err("Authentication denied. Reconnect this linked machine manually before transferring files.".into());
    }
    uuid::Uuid::parse_str(&uploaded.lease_id).map_err(|_| "Invalid staged upload identity.")?;
    if !(attachments::safe_id(&uploaded.attachment_id)
        || attachments::safe_file_id(&uploaded.attachment_id))
    {
        return Err("Invalid staged attachment identity.".into());
    }
    let result = http()?
        .get(format!(
            "http://{}/v1/staged/{}",
            link.address()?,
            uploaded.lease_id
        ))
        .bearer_auth(&link.token)
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .map_err(|_| "Uploaded file could not be verified. Retry when the server reconnects.")?;
    if result.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(false);
    }
    let result = match response(result).await {
        Ok(result) => result,
        Err(error) => {
            remember_denial(link, &error);
            return Err(error);
        }
    };
    let value = result
        .json::<Value>()
        .await
        .map_err(|_| "Uploaded file could not be verified. Retry when the server reconnects.")?;
    if value["id"] != uploaded.attachment_id {
        return Err(
            "Uploaded file could not be verified. Retry when the server reconnects.".into(),
        );
    }
    Ok(true)
}

/// Fetch a hydrated draft from its pending lease, never from a foreign filesystem path.
pub(crate) async fn download_staged(
    link: &Link,
    profile: &std::path::Path,
    attachment: &Attachment,
) -> Result<Attachment, String> {
    let uploaded = attachment
        .uploaded
        .as_ref()
        .ok_or("The attachment is unavailable.")?;
    if uploaded.environment_id != link.server_id || !verify_upload(link, uploaded).await? {
        return Err("Uploaded file expired. Remove it and attach it again.".into());
    }
    let maximum = if attachment.is_image() {
        attachments::MAX_IMAGE_BYTES
    } else {
        attachments::MAX_FILE_BYTES
    };
    if attachment.bytes == 0 || attachment.bytes > maximum {
        return Err("Invalid file length.".into());
    }
    let http = http()?;
    let mut bytes = Vec::new();
    while (bytes.len() as u64) < attachment.bytes {
        let result = response(
            http.get(format!(
                "http://{}/v1/staged/{}/blob?offset={}&limit={}",
                link.address()?,
                uploaded.lease_id,
                bytes.len(),
                protocol::PAGE_BYTES
            ))
            .bearer_auth(&link.token)
            .send()
            .await
            .map_err(|_| "The attachment is unavailable.")?,
        )
        .await;
        let response = match result {
            Ok(r) => r,
            Err(error) => {
                remember_denial(link, &error);
                return Err(error);
            }
        };
        if response
            .content_length()
            .is_none_or(|size| size > protocol::MAX_PAGE_BYTES as u64)
        {
            return Err("Invalid blob page length.".into());
        }
        let chunk = response
            .bytes()
            .await
            .map_err(|_| "Could not load this file.")?;
        if chunk.is_empty() || bytes.len() + chunk.len() > attachment.bytes as usize {
            return Err("Invalid remote file length.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let mut fetched = if attachment.is_image() {
        attachments::store_uploaded(&profile.join("attachments"), &attachment.name, &bytes)?
    } else {
        attachments::store_uploaded_file(
            &profile.join("attachments"),
            &attachment.name,
            &attachment.mime,
            &bytes,
        )?
    };
    if fetched.id != uploaded.attachment_id {
        return Err("Attachment hash mismatch.".into());
    }
    fetched.asset_id = Some(fetched.id.clone());
    fetched.id = attachment.id.clone();
    fetched.uploaded = Some(uploaded.clone());
    fetched.source = attachment.source.clone();
    Ok(fetched)
}

pub async fn release_upload(link: &Link, id: &str) {
    if authentication_denied(link) {
        return;
    }
    if let Ok(http) = http()
        && let Ok(address) = link.address()
    {
        let _ = http
            .delete(format!("http://{address}/v1/staged/{id}"))
            .bearer_auth(&link.token)
            .send()
            .await;
    }
}

fn release_later(link: Link, id: String) {
    std::thread::spawn(move || {
        if let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            runtime.block_on(release_upload(&link, &id));
        }
    });
}

pub async fn upload(
    link: &Link,
    attachment: &Attachment,
    cancel: &AtomicBool,
    progress: impl Fn(f32),
) -> Result<Uploaded, String> {
    if authentication_denied(link) {
        return Err("Authentication denied. Reconnect this linked machine manually before transferring files.".into());
    }
    let key = (link.server_id.clone(), attachment.id.clone());
    let generation = uuid::Uuid::new_v4();
    let (previous, slots) = {
        let mut registry = registry().lock().expect("attachment upload registry");
        let previous = registry.attempts.insert(
            key.clone(),
            Attempt {
                generation,
                lease: None,
            },
        );
        let slots = registry
            .slots
            .entry(link.server_id.clone())
            .or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(MAX_UPLOADS_PER_ENVIRONMENT)))
            .clone();
        (previous.and_then(|attempt| attempt.lease), slots)
    };
    if let Some(id) = previous {
        release_later(link.clone(), id);
    }
    let cancelled = async {
        while !cancel.load(Ordering::Acquire) {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    };
    let _permit = tokio::select! {
        permit = slots.acquire_owned() => permit.map_err(|_| "Upload could not start")?,
        _ = cancelled => {
            let mut registry = registry().lock().expect("attachment upload registry");
            if registry.attempts.get(&key).is_some_and(|attempt| attempt.generation == generation) {
                registry.attempts.remove(&key);
            }
            return Err("Upload cancelled".into());
        }
    };
    let result = upload_cycle(link, attachment, cancel, generation, progress).await;
    if let Err(error) = &result {
        remember_denial(link, error);
    }
    result
}

async fn upload_cycle(
    link: &Link,
    attachment: &Attachment,
    cancel: &AtomicBool,
    generation: uuid::Uuid,
    progress: impl Fn(f32),
) -> Result<Uploaded, String> {
    if authentication_denied(link) {
        return Err("Authentication denied. Reconnect this linked machine manually before transferring files.".into());
    }
    if cancel.load(Ordering::Acquire) {
        return Err("Upload cancelled".into());
    }
    let bytes = if attachment.is_image() {
        attachments::read(attachment)?
    } else {
        attachments::transfer_file_bytes(attachment)?
    };
    use sha2::Digest;
    let hash = format!("{:x}", sha2::Sha256::digest(&bytes));
    let http = http()?;
    let base = format!("http://{}", link.address()?);
    let minted = response(
        http.post(format!("{base}/v1/uploads"))
            .bearer_auth(&link.token)
            .json(
                &json!({"staged":true,"type":if attachment.is_image() {"image"} else {"file"},
            "name":attachment.name,"mimeType":attachment.mime,"length":bytes.len(),
            "sha256":hash}),
            )
            .send()
            .await
            .map_err(|_| "Upload could not start")?,
    )
    .await?
    .json::<Value>()
    .await
    .map_err(|_| "Invalid upload response.")?;
    let id = minted["uploadId"]
        .as_str()
        .filter(|id| uuid::Uuid::parse_str(id).is_ok())
        .ok_or("Invalid upload identity.")?;
    let key = (link.server_id.clone(), attachment.id.clone());
    let current = {
        let mut registry = registry().lock().expect("attachment upload registry");
        if let Some(attempt) = registry
            .attempts
            .get_mut(&key)
            .filter(|attempt| attempt.generation == generation)
        {
            attempt.lease = Some(id.into());
            true
        } else {
            false
        }
    };
    if !current || cancel.load(Ordering::Acquire) {
        release_later(link.clone(), id.into());
        return Err("Upload cancelled".into());
    }
    let operation = async {
        let mut offset = 0usize;
        let mut last_step = -1i32;
        for chunk in bytes.chunks(protocol::MAX_PAGE_BYTES) {
            if authentication_denied(link) {
                return Err("Authentication denied. Reconnect this linked machine manually before transferring files.".into());
            }
            if cancel.load(Ordering::Acquire) {
                return Err("Upload cancelled".into());
            }
            response(
                http.put(format!("{base}/v1/uploads/{id}?offset={offset}"))
                    .bearer_auth(&link.token)
                    .body(chunk.to_vec())
                    .send()
                    .await
                    .map_err(|_| "Upload failed")?,
            )
            .await?;
            offset += chunk.len();
            let fraction = offset as f32 / bytes.len() as f32;
            let step = (fraction * 20.0).floor() as i32;
            if step != last_step {
                progress(fraction);
                last_step = step;
            }
        }
        if cancel.load(Ordering::Acquire) {
            return Err("Upload cancelled".into());
        }
        if authentication_denied(link) {
            return Err("Authentication denied. Reconnect this linked machine manually before transferring files.".into());
        }
        let committed = response(
            http.post(format!("{base}/v1/uploads/{id}/commit"))
                .bearer_auth(&link.token)
                .send()
                .await
                .map_err(|_| "Upload failed")?,
        )
        .await?
        .json::<Value>()
        .await
        .map_err(|_| "Invalid committed attachment.")?;
        let attachment_id = committed["id"]
            .as_str()
            .ok_or("Invalid attachment identity.")?;
        if !(attachments::safe_id(attachment_id) || attachments::safe_file_id(attachment_id)) {
            return Err("Invalid attachment identity.".into());
        }
        if attachment.is_image() {
            if !attachment_id.starts_with(&format!("{hash}.")) {
                return Err("Image hash mismatch.".into());
            }
        } else if attachment_id != format!("file-{hash}.bin") {
            return Err("File hash mismatch.".into());
        }
        Ok(Uploaded {
            environment_id: link.server_id.clone(),
            lease_id: id.into(),
            attachment_id: attachment_id.into(),
        })
    };
    let cancellation = async {
        while !cancel.load(Ordering::Acquire) {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    };
    let result = tokio::select! {
        result = tokio::time::timeout(UPLOAD_TIMEOUT, operation) =>
            result.map_err(|_| "Upload timed out".to_owned()).and_then(|result| result),
        _ = cancellation => Err("Upload cancelled".into()),
    };
    if let Err(error) = &result {
        remember_denial(link, error);
    }
    let mut registry = registry().lock().expect("attachment upload registry");
    let current = registry
        .attempts
        .get(&key)
        .is_some_and(|attempt| attempt.generation == generation);
    if result.is_ok() || cancel.load(Ordering::Acquire) {
        if current {
            registry.attempts.remove(&key);
        }
    }
    drop(registry);
    if cancel.load(Ordering::Acquire) || !current {
        release_later(link.clone(), id.into());
        Err("Upload cancelled".into())
    } else {
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn advertised_file_limits_bound_intake_paste_and_existing_drafts() {
        let mut state = State::new(".".into(), false);
        let (client, _) = crate::client::Client::test_connected(state.data_dir.clone());
        *client.info.lock().unwrap() = json!({"capabilities":[protocol::FILE_ATTACHMENTS,
            protocol::ATTACHMENT_LIMITS,protocol::PASTED_TEXT_ATTACHMENTS,protocol::STAGED_ATTACHMENTS],
            "limits":{"fileAttachmentBytes":1024}});
        state.client = Some(client);
        let path = state.data_dir.join("one.bin");
        std::fs::write(&path, vec![1; 1025]).unwrap();
        state.attach_paths_to(state.current().id, &[path.clone()]);
        assert!(state.current().draft_images.is_empty());
        assert_eq!(
            state.current().attachment_error.as_deref(),
            Some("'one.bin' exceeds the 1 KB attachment limit.")
        );
        assert!(!state.fold_paste(&"x".repeat(crate::composer_paste::THRESHOLD)));
        state
            .current_mut()
            .draft_images
            .push(attachments::attach_file(&path).unwrap());
        assert_eq!(
            state.attachment_send_block_reason().as_deref(),
            Some("'one.bin' exceeds the 1 KB attachment limit.")
        );
        let mut queue = Queue::default();
        queue.entries.push(Entry::fixture(
            "local",
            state.current().id,
            &state.current().draft_images[0],
            Phase::Uploading {
                progress: 0.5,
                since: Instant::now(),
            },
        ));
        let cancelled = queue.entries[0].cancel.clone();
        assert!(queue.poll(&mut state));
        assert!(cancelled.load(Ordering::Acquire));
        assert!(matches!(&queue.entries[0].phase, Phase::Failed(reason)
            if reason == "'one.bin' exceeds the 1 KB attachment limit."));
        *state.client.as_ref().unwrap().info.lock().unwrap() =
            json!({"capabilities":[protocol::FILE_ATTACHMENTS]});
        assert_eq!(
            state.file_attachment_staging_limit_on("local"),
            Some(attachments::MAX_FILE_BYTES)
        );
        assert!(state.attachment_send_block_reason().is_none());
    }

    #[test]
    fn an_older_peer_requires_reattach_for_a_pending_only_restored_file() {
        let mut state = State::new(".".into(), false);
        let path = state.data_dir.join("pending-only.txt");
        std::fs::write(&path, "original").unwrap();
        let mut file = attachments::attach_file(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        file.uploaded = Some(Uploaded {
            environment_id: "fixture".into(),
            lease_id: uuid::Uuid::new_v4().to_string(),
            attachment_id: format!("file-{}.bin", "0".repeat(64)),
        });
        state.current_mut().draft_images.push(file);
        let (client, _) = crate::client::Client::test_connected(state.data_dir.clone());
        *client.info.lock().unwrap() = json!({"capabilities":[protocol::FILE_ATTACHMENTS]});
        state.client = Some(client);
        assert!(
            state
                .attachment_send_block_reason()
                .unwrap()
                .contains("does not support staged attachments")
        );
        assert!(
            state.current().draft_images[0].uploaded.is_some(),
            "capability loss does not release the persisted lease"
        );
    }
    #[test]
    fn preparing_and_prepared_tiles_share_the_source_thumbnail_handle() {
        let mut state = State::new(".".into(), false);
        let mut png = std::io::Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(2, 2, image::Rgba([10, 20, 30, 255]))
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let png = png.into_inner();
        let attachment = attachments::attach_bytes("source.png", png.clone()).unwrap();
        let mut entry = Entry::fixture("local", 1, &attachment, Phase::Preparing);
        let (send, receive) = mpsc::channel();
        entry.worker = Some(receive);
        let mut queue = Queue::default();
        queue.entries.push(entry);
        send.send(Event::Thumbnail(png)).unwrap();
        queue.poll(&mut state);
        let first = queue.entries[0].thumbnail(&state.attachment_cache).unwrap();
        send.send(Event::Prepared(Ok(attachment))).unwrap();
        drop(send);
        queue.poll(&mut state);
        let second = queue.entries[0].thumbnail(&state.attachment_cache).unwrap();
        assert_eq!(first.id(), second.id());
        assert_eq!(second.wait().unwrap(), [2, 2]);
    }
    #[test]
    fn progress_is_floored_bounded_and_hidden_for_150_ms() {
        assert_eq!(progress_label(f32::NAN), "0%");
        assert_eq!(progress_label(-1.0), "0%");
        assert_eq!(progress_label(0.499), "49%");
        assert_eq!(progress_label(2.0), "100%");
        let phase = Phase::Uploading {
            progress: 0.499,
            since: std::time::Instant::now(),
        };
        assert_eq!(phase.label(std::time::Duration::from_millis(149)), None);
        assert_eq!(
            phase.label(std::time::Duration::from_millis(150)),
            Some("49%".into())
        );
    }
    #[test]
    fn failures_allow_send_retry_but_preparation_and_upload_block_it() {
        assert!(Phase::Preparing.blocks_send());
        assert!(Phase::Queued.blocks_send());
        assert!(!Phase::Ready.blocks_send());
        assert!(!Phase::Failed("Upload failed".into()).blocks_send());
        assert_eq!(Capability::from_caps(None), Capability::Unknown);
        assert_eq!(Capability::from_caps(Some(&[])), Capability::Legacy);
        assert_eq!(
            Capability::from_caps(Some(&[serde_json::json!(
                crate::protocol::STAGED_ATTACHMENTS
            )])),
            Capability::Staged
        );
    }

    #[test]
    fn capability_loss_cancels_a_live_upload_and_regain_queues_it_again() {
        let mut state = State::new(".".into(), false);
        let (client, _commands) = crate::client::Client::test_connected(state.data_dir.clone());
        *client.info.lock().unwrap() = json!({"capabilities":[]});
        state.client = Some(client);
        let attachment = Attachment {
            id: "capability.png".into(),
            name: "photo.png".into(),
            mime: "image/png".into(),
            width: 1,
            height: 1,
            bytes: 1,
            path: None,
            asset_id: None,
            uploaded: None,
            source: None,
        };
        state.current_mut().draft_images.push(attachment.clone());
        let mut entry = Entry::fixture(
            "local",
            1,
            &attachment,
            Phase::Uploading {
                progress: 0.5,
                since: Instant::now(),
            },
        );
        let cancel = entry.cancel.clone();
        let (_send, receive) = mpsc::channel();
        entry.worker = Some(receive);
        let mut queue = Queue::default();
        queue.entries.push(entry);
        assert!(queue.poll(&mut state));
        assert!(cancel.load(Ordering::Acquire));
        assert!(matches!(queue.entries[0].phase, Phase::Legacy));
        assert!(queue.block_reason("local", 1).is_none());
        *state.client.as_ref().unwrap().info.lock().unwrap() =
            json!({"capabilities":[protocol::STAGED_ATTACHMENTS]});
        queue.poll(&mut state);
        assert!(matches!(queue.entries[0].phase, Phase::Queued));
        assert!(queue.block_reason("local", 1).is_some());
    }

    #[test]
    fn preparation_reserves_slots_and_removed_results_cannot_restore_a_tile() {
        let mut state = State::new(".".into(), false);
        let mut queue = Queue::default();
        for index in 0..100 {
            queue.enqueue(
                &mut state,
                0,
                Input::Image(format!("{index}.png"), vec![1]),
                false,
            );
        }
        assert_eq!(queue.preparing_count("local", 1), 100);
        queue.enqueue(
            &mut state,
            0,
            Input::Image("overflow.png".into(), vec![1]),
            false,
        );
        assert_eq!(queue.entries.len(), 100);
        assert_eq!(
            state.current().attachment_error.as_deref(),
            Some("You can attach up to 100 files per message.")
        );
        let id = queue.entries[0].id.clone();
        let cancel = queue.entries[0].cancel.clone();
        let (send, receive) = mpsc::channel();
        queue.entries[0].worker = Some(receive);
        queue.remove("local", 1, &id);
        assert!(
            send.send(Event::Prepared(Err("late failure".into())))
                .is_err()
        );
        assert!(cancel.load(Ordering::Acquire));
        assert!(!queue.entries.iter().any(|entry| entry.id == id));
        assert_eq!(MAX_UPLOADS_PER_ENVIRONMENT, 3);
        assert_eq!(UPLOAD_TIMEOUT, Duration::from_secs(300));
    }

    #[test]
    fn prepared_files_keep_the_origin_environment_and_persist_inactive_drafts() {
        let mut state = State::new(".".into(), false);
        state.current_mut().draft.replace_text("Local draft");
        let mut remote = crate::model::Thread::new(1);
        remote.machine = "remote".into();
        remote.draft.replace_text("Remote draft");
        state.machines.threads.insert("remote".into(), vec![remote]);
        state.machines.ui["remote"] = json!({"drafts":{}});
        let path = state.data_dir.join("origin.txt");
        std::fs::write(&path, "owned bytes").unwrap();
        state.attach_paths_on_to("remote", 1, &[path.clone()]);
        let mut queue = std::mem::take(&mut state.attachment_uploads);
        let id = queue.entries[0].id.clone();
        let owned =
            attachments::stage_transfer_file(&attachments::attach_file(&path).unwrap()).unwrap();
        let (send, receive) = mpsc::channel();
        queue.entries[0].worker = Some(receive);
        send.send(Event::Prepared(Ok(owned))).ok().unwrap();
        drop(send);
        queue.poll(&mut state);
        assert_eq!(state.current().draft.text, "Local draft");
        assert!(state.current().draft_images.is_empty());
        let remote = &state.machines.threads["remote"][0];
        assert_eq!(remote.draft_images[0].id, id);
        assert_eq!(remote.draft_images[0].name, "origin.txt");
        assert_eq!(
            state.machines.ui["remote"]["drafts"]["1"]["images"][0]["id"],
            id
        );
        assert!(remote.draft_images[0].asset_id.is_some());
        assert_eq!(
            Attachment::from_value(&remote.draft_images[0].to_value()),
            Some(remote.draft_images[0].clone())
        );
    }

    #[test]
    fn authentication_denial_stops_queued_requests_until_credentials_change() {
        let mut state = State::new(".".into(), false);
        let (mut client, _commands) = crate::client::Client::test_connected(state.data_dir.clone());
        let link = Link {
            server_id: uuid::Uuid::new_v4().to_string(),
            host: "127.0.0.1:1".into(),
            device_id: uuid::Uuid::new_v4().to_string(),
            token: "fixture-old".into(),
        };
        client.remote = Some(link.clone());
        *client.info.lock().unwrap() =
            json!({"capabilities":[protocol::STAGED_ATTACHMENTS,protocol::ATTACHMENT_LIMITS]});
        state.client = Some(client);
        let image = Attachment {
            id: "denial.png".into(),
            name: "denial.png".into(),
            mime: "image/png".into(),
            width: 1,
            height: 1,
            bytes: 1,
            path: None,
            asset_id: None,
            uploaded: None,
            source: None,
        };
        state.current_mut().draft_images.push(image.clone());
        let mut queue = Queue::default();
        queue
            .entries
            .push(Entry::fixture("local", 1, &image, Phase::Queued));
        remember_denial(&link, "Authentication denied. Reconnect manually.");
        queue.poll(&mut state);
        assert!(queue.entries[0].worker.is_none());
        assert!(
            matches!(&queue.entries[0].phase, Phase::Failed(reason) if reason.starts_with("Authentication denied."))
        );
        let client = state.client.as_mut().unwrap();
        client.remote.as_mut().unwrap().token = "fixture-new".into();
        *client.info.lock().unwrap() = json!({"capabilities":[]});
        queue.poll(&mut state);
        assert!(matches!(queue.entries[0].phase, Phase::Legacy));
        assert!(queue.block_reason("local", 1).is_none());
    }
}
