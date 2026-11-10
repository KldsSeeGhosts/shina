//! Global project records, legacy recents, and background cloning.
//!
//! Projects persist in Shina's global index. The old `projects.json` and
//! `last-workspace.txt` are migration inputs only. Cloning runs `gh` or `git` on a
//! worker thread with no console window and no stdin; it relies on the user's existing
//! login and never asks for, reads, or stores a credential.
use crate::model::State;
use rune::{TextEdit, Tx, Waker};
use serde_json::{Value, json};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        mpsc::{Receiver, channel},
    },
    time::{SystemTime, UNIX_EPOCH},
};

/// How many projects the list keeps. Older entries fall off the end.
#[cfg(test)]
const LIMIT: usize = 20;

pub type ProjectId = usize;

#[derive(Clone, Debug)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub path: PathBuf,
    pub machine: String,
    /// Removing a project hides its picker entry, not its threads or files.
    pub hidden: bool,
    pub updated: u64,
    pub captain: Option<usize>,
    /// The designated coordinator. Older home flags remain historical metadata.
    pub captain_thread: Option<usize>,
    /// A PNG icon found in the project, shown beside its threads as T3 Code shows favicons.
    pub icon: Option<PathBuf>,
}

impl Project {
    pub fn new(id: ProjectId, path: PathBuf) -> Self {
        let path = normalize_path(&path);
        Self {
            id,
            name: display_name(&path),
            machine: "local".into(),
            hidden: false,
            updated: now(),
            captain: None,
            captain_thread: None,
            icon: find_icon(&path),
            path,
        }
    }
    pub fn to_value(&self) -> Value {
        json!({
            "id": self.id, "name": self.name, "path": self.path, "machine": self.machine,
            "hidden": self.hidden, "updated": self.updated,
            "captain": self.captain, "captainThread": self.captain_thread,
        })
    }
    pub fn from_value(value: &Value) -> Option<Self> {
        let id = usize::try_from(value["id"].as_u64()?).ok()?;
        if id == 0 || id == usize::MAX {
            return None;
        }
        let mut project = Self::new(id, PathBuf::from(value["path"].as_str()?));
        project.name = value["name"].as_str()?.into();
        project.machine = value["machine"].as_str()?.into();
        if project.path.as_os_str().is_empty() || project.machine.is_empty() {
            return None;
        }
        if project.machine != "local" {
            project.icon = None;
        }
        project.hidden = value["hidden"] == true;
        project.updated = value["updated"].as_u64().unwrap_or(0);
        project.captain = optional_id(&value["captain"])?;
        project.captain_thread = optional_id(&value["captainThread"])?;
        Some(project)
    }
}

/// The first PNG icon a project ships, in the places apps usually keep one. Shina's image
/// decoder reads PNG, so ICO and SVG favicons are skipped.
fn find_icon(root: &Path) -> Option<PathBuf> {
    [
        "assets/project-icon.png",
        "favicon.png",
        "public/favicon.png",
        "static/favicon.png",
        "assets/favicon.png",
        "app/icon.png",
        "src/app/icon.png",
        "public/icon.png",
        "public/logo.png",
        "assets/icon.png",
        "assets/logo.png",
        "icon.png",
        "logo.png",
    ]
    .iter()
    .map(|candidate| root.join(candidate))
    .find(|path| path.is_file())
}

fn optional_id(value: &Value) -> Option<Option<usize>> {
    if value.is_null() {
        Some(None)
    } else {
        Some(Some(usize::try_from(value.as_u64()?).ok()?))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RecentProject {
    pub path: PathBuf,
    pub name: String,
    /// Seconds since the Unix epoch.
    pub opened: u64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Recents {
    /// Most recently opened first.
    #[cfg(test)]
    pub projects: Vec<RecentProject>,
    /// The parent folder the last successful clone went into.
    pub clone_parent: Option<PathBuf>,
}

pub fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Strip only filesystem verbatim disk/UNC prefixes, not Windows device namespaces.
pub fn clean_path(path: &Path) -> PathBuf {
    if let Some(text) = path.to_str() {
        if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{unc}"));
        }
        if let Some(disk) = text.strip_prefix(r"\\?\")
            && disk.as_bytes().get(1..3) == Some(b":\\")
        {
            return PathBuf::from(disk);
        }
    }
    path.to_path_buf()
}

pub fn normalize_path(path: &Path) -> PathBuf {
    clean_path(&path.canonicalize().unwrap_or_else(|_| clean_path(path)))
}

pub fn same_path(left: &Path, right: &Path) -> bool {
    let left = normalize_path(left);
    let right = normalize_path(right);
    if cfg!(windows) {
        left.as_os_str().eq_ignore_ascii_case(right.as_os_str())
    } else {
        left == right
    }
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl Recents {
    /// Moves `path` to the front, adding it if it is new.
    #[cfg(test)]
    pub fn touch(&mut self, path: &Path, opened: u64) {
        self.projects.retain(|project| project.path != path);
        self.projects.insert(
            0,
            RecentProject {
                path: path.to_path_buf(),
                name: display_name(path),
                opened,
            },
        );
        self.projects.truncate(LIMIT);
    }
    /// The projects whose folders still exist. Missing ones stay in the file, so a folder on
    /// an unplugged drive comes back when the drive does.
    #[cfg(test)]
    pub fn available(&self) -> Vec<RecentProject> {
        self.projects
            .iter()
            .filter(|project| project.path.is_dir())
            .cloned()
            .collect()
    }
    #[cfg(test)]
    pub fn last_existing(&self) -> Option<PathBuf> {
        self.projects
            .iter()
            .map(|project| project.path.clone())
            .find(|path| path.is_dir())
    }
    #[cfg(test)]
    fn to_value(&self) -> Value {
        json!({
            "version": 1,
            "cloneParent": self.clone_parent.as_ref().map(|p| p.to_string_lossy().into_owned()),
            "projects": self.projects.iter().map(|p| json!({
                "path": p.path.to_string_lossy(), "name": p.name, "opened": p.opened,
            })).collect::<Vec<_>>(),
        })
    }
    #[cfg(test)]
    fn from_value(value: &Value) -> Self {
        let projects = value["projects"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|record| {
                let path = PathBuf::from(record["path"].as_str()?);
                Some(RecentProject {
                    name: record["name"]
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| display_name(&path)),
                    path,
                    opened: record["opened"].as_u64().unwrap_or(0),
                })
            })
            .take(LIMIT)
            .collect();
        Self {
            projects,
            clone_parent: value["cloneParent"].as_str().map(PathBuf::from),
        }
    }
}

/// Reads the list from `dir`, seeding it from `last-workspace.txt` the first time.
#[cfg(test)]
pub fn load_from(dir: &Path) -> Recents {
    if let Ok(text) = fs::read_to_string(dir.join("projects.json")) {
        return serde_json::from_str(&text)
            .map(|value| Recents::from_value(&value))
            .unwrap_or_default();
    }
    let mut recents = Recents::default();
    let legacy = dir.join("last-workspace.txt");
    if let Ok(text) = fs::read_to_string(&legacy) {
        let path = PathBuf::from(text.trim());
        if !path.as_os_str().is_empty() {
            let opened = fs::metadata(&legacy)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs());
            recents.touch(&path, opened);
            let _ = save_to(dir, &recents);
        }
    }
    recents
}

/// Writes the list through a temporary file, so a crash never leaves half a file.
#[cfg(test)]
pub fn save_to(dir: &Path, recents: &Recents) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    let staged = dir.join("projects.json.tmp");
    fs::write(&staged, serde_json::to_vec_pretty(&recents.to_value())?)?;
    fs::rename(staged, dir.join("projects.json"))
}

/// What the clone field holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// `owner/repo`, cloned with `gh repo clone`.
    GitHub { owner: String, repo: String },
    /// Any other git URL, cloned with `git clone`.
    Url(String),
}

fn shorthand_part(part: &str) -> bool {
    !part.is_empty()
        && part != "."
        && part != ".."
        && part
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

pub fn parse_source(input: &str) -> Result<Source, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("Enter a GitHub URL, a git URL, or owner/repo.".into());
    }
    if input.starts_with('-') || input.chars().any(char::is_whitespace) {
        return Err("That does not look like a repository. Use a URL or owner/repo.".into());
    }
    if let Some((owner, repo)) = input.split_once('/')
        && !input.contains(':')
        && shorthand_part(owner)
        && shorthand_part(repo)
    {
        return Ok(Source::GitHub {
            owner: owner.into(),
            repo: repo.strip_suffix(".git").unwrap_or(repo).into(),
        });
    }
    if let Some((scheme, rest)) = input.split_once("://") {
        let scheme = scheme.to_ascii_lowercase();
        if !matches!(
            scheme.as_str(),
            "https" | "http" | "ssh" | "git" | "git+ssh" | "file"
        ) {
            return Err(format!("Shina cannot clone {scheme}:// URLs."));
        }
        let path = rest.split_once('/').map(|(_, path)| path).unwrap_or("");
        if path.trim_matches('/').is_empty() {
            return Err("The URL needs a repository path after the host.".into());
        }
        return Ok(Source::Url(input.into()));
    }
    // scp-like syntax, `git@github.com:owner/repo.git`. A single letter before the colon is a
    // Windows drive, not a host.
    if let Some((host, path)) = input.split_once(':')
        && host.len() > 1
        && !host.contains('/')
        && !host.contains('\\')
        && !path.trim_matches('/').is_empty()
    {
        return Ok(Source::Url(input.into()));
    }
    Err("That does not look like a repository. Use a URL or owner/repo.".into())
}

impl Source {
    /// The folder name the clone gets, as `git clone` would choose it.
    pub fn folder_name(&self) -> Result<String, String> {
        let raw = match self {
            Self::GitHub { repo, .. } => repo.as_str(),
            Self::Url(url) => {
                let url = url.split(['?', '#']).next().unwrap_or(url);
                let url = url.trim_end_matches('/');
                let url = url.strip_suffix(".git").unwrap_or(url);
                url.rsplit(['/', ':']).next().unwrap_or("")
            }
        };
        let name = raw.strip_suffix(".git").unwrap_or(raw);
        let invalid = name.is_empty()
            || name == "."
            || name == ".."
            || name.ends_with(['.', ' '])
            || name
                .chars()
                .any(|c| c.is_control() || r#"<>:"/\|?*"#.contains(c));
        if invalid {
            return Err("Shina could not work out a folder name for that repository.".into());
        }
        Ok(name.into())
    }
    /// The text to show for this source, without any user name or password in the URL.
    pub fn display(&self) -> String {
        match self {
            Self::GitHub { owner, repo } => format!("{owner}/{repo}"),
            Self::Url(url) => redact(url),
        }
    }
}

/// Removes `user:password@` from every URL in `text`.
pub fn redact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(index) = rest.find("://") {
        let (head, tail) = rest.split_at(index + 3);
        out.push_str(head);
        let end = tail
            .find(|c: char| c == '/' || c.is_whitespace() || c == '\'' || c == '"')
            .unwrap_or(tail.len());
        match tail[..end].rfind('@') {
            Some(at) => rest = &tail[at + 1..],
            None => rest = tail,
        }
    }
    out.push_str(rest);
    out
}

/// Where a clone of `name` into `parent` goes, refusing anything git would refuse or that
/// would mix the clone with existing files.
pub fn destination(parent: &Path, name: &str) -> Result<PathBuf, String> {
    if !parent.is_dir() {
        return Err(format!(
            "The destination folder {} does not exist.",
            parent.display()
        ));
    }
    let target = parent.join(name);
    if target.exists() {
        if !target.is_dir() {
            return Err(format!(
                "{} already exists and is not a folder.",
                target.display()
            ));
        }
        let empty = fs::read_dir(&target)
            .map_err(|e| format!("Could not read {}: {e}", target.display()))?
            .next()
            .is_none();
        if !empty {
            return Err(format!(
                "{} already exists and is not empty. Choose another destination.",
                target.display()
            ));
        }
    }
    Ok(target)
}

fn hidden(command: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    command
}

/// The command a clone runs. Prompts are turned off, so a missing login fails with git's
/// message instead of waiting for input or opening a sign-in window.
pub fn clone_command(source: &Source, target: &Path) -> Command {
    let mut command = match source {
        Source::GitHub { owner, repo } => {
            let mut command = Command::new("gh");
            command
                .args(["repo", "clone", &format!("{owner}/{repo}")])
                .arg(target)
                .args(["--", "--progress"]);
            command
        }
        Source::Url(url) => {
            let mut command = Command::new("git");
            command.args(["clone", "--progress", "--", url]).arg(target);
            command
        }
    };
    command
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "never")
        .env("GH_PROMPT_DISABLED", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    hidden(&mut command);
    command
}

#[cfg(test)]
impl CloneJob {
    /// Completion followed by late progress, so one poll reads both in this order.
    pub fn test_finished(progress: &str, result: Result<(), String>) -> Self {
        let (tx, events) = channel();
        let _ = tx.send(CloneEvent::Done(result));
        let _ = tx.send(CloneEvent::Progress(progress.into()));
        Self::test_idle_with(events)
    }

    /// A clone that is running but never finishes, for guards and UI tests.
    pub fn test_idle() -> Self {
        let (tx, events) = channel();
        std::mem::forget(tx);
        Self::test_idle_with(events)
    }

    fn test_idle_with(events: Receiver<CloneEvent>) -> Self {
        Self {
            events,
            child: Arc::new(Mutex::new(None)),
            canceled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            target: PathBuf::new(),
            parent: PathBuf::new(),
        }
    }
}

enum CloneEvent {
    Progress(String),
    Done(Result<(), String>),
}

pub struct CloneJob {
    events: Receiver<CloneEvent>,
    child: Arc<Mutex<Option<Child>>>,
    canceled: Arc<std::sync::atomic::AtomicBool>,
    pub target: PathBuf,
    pub parent: PathBuf,
}

impl CloneJob {
    fn start(source: &Source, parent: PathBuf, target: PathBuf, wake: Waker) -> Self {
        let (tx, events) = channel();
        let child = Arc::new(Mutex::new(None));
        let canceled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut command = clone_command(source, &target);
        let tool = if matches!(source, Source::GitHub { .. }) {
            "gh"
        } else {
            "git"
        };
        let (slot, stop) = (child.clone(), canceled.clone());
        std::thread::spawn(move || {
            let send = |event| {
                let _ = tx.send(event);
                wake();
            };
            let mut process = match command.spawn() {
                Ok(process) => process,
                Err(error) => {
                    send(CloneEvent::Done(Err(format!(
                        "Could not start {tool}: {error}. Is it installed and on PATH?"
                    ))));
                    return;
                }
            };
            let mut stderr = process.stderr.take();
            *slot.lock().unwrap() = Some(process);
            let mut tail: Vec<String> = vec![];
            let mut line = Vec::new();
            let mut buffer = [0u8; 1024];
            while let Some(Ok(read)) = stderr.as_mut().map(|s| s.read(&mut buffer)) {
                if read == 0 {
                    break;
                }
                for &byte in &buffer[..read] {
                    if byte == b'\r' || byte == b'\n' {
                        let text = redact(String::from_utf8_lossy(&line).trim());
                        line.clear();
                        if !text.is_empty() {
                            send(CloneEvent::Progress(text.clone()));
                            if tail.len() == 6 {
                                tail.remove(0);
                            }
                            tail.push(text);
                        }
                    } else {
                        line.push(byte);
                    }
                }
            }
            let process = slot.lock().unwrap().take();
            let status = process.map(|mut p| p.wait());
            let result = if stop.load(std::sync::atomic::Ordering::SeqCst) {
                Err("Clone canceled.".into())
            } else {
                match status {
                    Some(Ok(status)) if status.success() => Ok(()),
                    _ => {
                        // Progress lines repeat; the failure reason is in the last few.
                        let reason: Vec<_> =
                            tail.iter().filter(|l| !l.contains('%')).cloned().collect();
                        Err(if reason.is_empty() {
                            format!("{tool} could not clone the repository.")
                        } else {
                            reason.join("\n")
                        })
                    }
                }
            };
            send(CloneEvent::Done(result));
        });
        Self {
            events,
            child,
            canceled,
            target,
            parent,
        }
    }
    fn cancel(&self) {
        self.canceled
            .store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(child) = self.child.lock().unwrap().as_mut() {
            let _ = child.kill();
        }
    }
}

/// The switcher menu and the clone sheet.
#[derive(Default)]
pub struct Projects {
    pub entries: Vec<Project>,
    pub next_id: ProjectId,
    pub captain_target: Option<ProjectId>,
    pub recents: Recents,
    /// The projects the open menu lists, checked for existence when it opened.
    pub menu: Option<Vec<RecentProject>>,
    pub clone_open: bool,
    pub clone_source: TextEdit,
    pub clone_parent: PathBuf,
    pub clone_progress: String,
    pub clone_error: Option<String>,
    pub clone_job: Option<CloneJob>,
    /// Shown in the menu when opening a project fails, such as when a thread is busy.
    pub menu_error: Option<String>,
}

impl Projects {
    /// A fixed list for headless renders.
    pub fn demo() -> Self {
        Self {
            menu: None,
            clone_parent: PathBuf::from(r"D:\AiStack"),
            ..Default::default()
        }
    }
    pub fn cloning(&self) -> bool {
        self.clone_job.is_some()
    }
}

impl State {
    pub fn project_name(&self) -> String {
        self.project(self.current().project)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "No project".into())
    }
    pub fn open_project_menu(&mut self) {
        self.projects.menu_error = None;
        self.projects.menu = Some(
            self.projects
                .entries
                .iter()
                .filter(|p| p.hidden)
                .map(|p| RecentProject {
                    name: p.name.clone(),
                    path: p.path.clone(),
                    opened: p.updated,
                })
                .collect(),
        );
        self.projects
            .menu
            .as_mut()
            .unwrap()
            .sort_by_key(|p| std::cmp::Reverse(p.opened));
    }
    pub fn close_project_menu(&mut self) {
        self.projects.menu = None;
        self.projects.menu_error = None;
    }
    /// Opens a project from the menu, keeping the menu open with the reason if it cannot.
    pub fn open_project(&mut self, path: PathBuf) {
        if let Err(error) = self.add_project(path) {
            self.projects.menu_error = Some(error);
        } else {
            self.close_project_menu();
        }
    }
    pub fn open_clone_dialog(&mut self) {
        self.projects.menu = None;
        self.projects.clone_open = true;
        self.projects.clone_error = None;
        self.projects.clone_progress.clear();
        self.projects.clone_source.focused = true;
        if self.demo {
            return;
        }
        self.projects.clone_parent = self
            .projects
            .recents
            .clone_parent
            .clone()
            .filter(|p| p.is_dir())
            .or_else(|| self.working_directory().parent().map(Path::to_path_buf))
            .unwrap_or_else(|| self.data_dir.clone());
    }
    pub fn close_clone_dialog(&mut self) {
        if let Some(job) = &self.projects.clone_job {
            job.cancel();
            return;
        }
        self.projects.clone_open = false;
        self.projects.clone_source.focused = false;
    }
    /// The folder the clone would create, or why it cannot.
    pub fn clone_target(&self) -> Result<(Source, PathBuf), String> {
        let source = parse_source(&self.projects.clone_source.text)?;
        let name = source.folder_name()?;
        Ok((source, self.projects.clone_parent.join(name)))
    }
    pub fn start_clone(&mut self) {
        if self.projects.cloning() {
            return;
        }
        let result = parse_source(&self.projects.clone_source.text).and_then(|source| {
            let name = source.folder_name()?;
            let target = destination(&self.projects.clone_parent, &name)?;
            Ok((source, target))
        });
        let (source, target) = match result {
            Ok(found) => found,
            Err(error) => {
                self.projects.clone_error = Some(error);
                return;
            }
        };
        self.projects.clone_error = None;
        self.projects.clone_progress = format!("Cloning {}...", source.display());
        if self.demo {
            return;
        }
        self.projects.clone_job = Some(CloneJob::start(
            &source,
            self.projects.clone_parent.clone(),
            target,
            self.wake.clone(),
        ));
    }
    /// Drains clone progress. Returns whether anything changed.
    pub fn poll_projects(&mut self) -> bool {
        let Some(job) = &self.projects.clone_job else {
            return false;
        };
        let mut changed = false;
        let mut finished = None;
        while let Ok(event) = job.events.try_recv() {
            changed = true;
            match event {
                // A clone that reported its result has already taken the project in.
                CloneEvent::Progress(_) if finished.is_some() => {}
                CloneEvent::Progress(line) => self.projects.clone_progress = line,
                CloneEvent::Done(result) => finished = Some(result),
            }
        }
        let Some(result) = finished else {
            return changed;
        };
        let job = self.projects.clone_job.take().unwrap();
        match result {
            Ok(()) => {
                self.projects.recents.clone_parent = Some(job.parent.clone());
                let target = job.target.clone();
                if let Err(error) = self.add_project(target) {
                    self.projects.clone_progress.clear();
                    self.projects.clone_error = Some(format!(
                        "Cloned into {}, but Shina could not open it yet. {error}",
                        job.target.display()
                    ));
                } else {
                    self.projects.clone_open = false;
                    self.projects.clone_source.focused = false;
                    self.checkpoint();
                }
            }
            Err(error) => {
                self.projects.clone_progress.clear();
                let canceled = error == "Clone canceled.";
                self.projects.clone_error = Some(if canceled && job.target.exists() {
                    format!(
                        "Clone canceled. Delete {} before cloning there again.",
                        job.target.display()
                    )
                } else {
                    error
                });
            }
        }
        true
    }
}

/// The native folder picker, shared by the menu and Ctrl+O.
pub fn pick_folder(s: &mut State, tx: &mut Tx) {
    s.projects.menu = None;
    tx.open_folder(
        rune::FileDialogOptions {
            title: "Open a project folder".into(),
            directory: Some(s.working_directory()),
            ..Default::default()
        },
        |s: &mut State, result, _| match result {
            Ok(Some(path)) => {
                if let Err(error) = s.add_project(path) {
                    s.current_mut().error = Some(error);
                }
            }
            Ok(None) => {}
            Err(error) => {
                s.current_mut().error = Some(format!("Could not open the folder picker: {error}"))
            }
        },
    );
}

/// The folder picker for the clone destination.
pub fn pick_clone_parent(s: &mut State, tx: &mut Tx) {
    tx.open_folder(
        rune::FileDialogOptions {
            title: "Choose where to clone".into(),
            directory: Some(s.projects.clone_parent.clone()),
            ..Default::default()
        },
        |s: &mut State, result, _| match result {
            Ok(Some(path)) => {
                s.projects.clone_parent = path;
                s.projects.clone_error = None;
            }
            Ok(None) => {}
            Err(error) => {
                s.projects.clone_error = Some(format!("Could not open the folder picker: {error}"))
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filesystem_paths_drop_verbatim_prefixes_without_changing_devices() {
        assert_eq!(
            clean_path(Path::new(r"\\?\D:\AiStack\shina")),
            PathBuf::from(r"D:\AiStack\shina")
        );
        assert_eq!(
            clean_path(Path::new(r"\\?\UNC\server\share\repo")),
            PathBuf::from(r"\\server\share\repo")
        );
        assert_eq!(
            clean_path(Path::new(r"\\?\GLOBALROOT\Device\HarddiskVolume1")),
            PathBuf::from(r"\\?\GLOBALROOT\Device\HarddiskVolume1")
        );
    }

    #[test]
    fn a_finished_clone_is_taken_in_once_even_with_progress_behind_its_result() {
        let dir = temp("clone-finish");
        let target = dir.join("repo");
        fs::create_dir_all(&target).unwrap();
        let mut state = crate::model::State::new(dir.clone(), false);
        state.projects.entries.clear();
        let mut job = CloneJob::test_finished("Receiving objects: 100%", Ok(()));
        job.target = target.clone();
        job.parent = dir.clone();
        state.projects.clone_job = Some(job);
        // The first poll takes the clone in; a late progress line must not repeat that.
        state.poll_projects();
        assert!(state.projects.clone_job.is_none());
        assert!(
            state
                .projects
                .entries
                .iter()
                .any(|project| same_path(&project.path, &target))
        );
        assert!(!state.poll_projects());
        let _ = fs::remove_dir_all(dir);
    }

    fn temp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("shina-projects-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn recents_round_trip_and_move_to_front() {
        let dir = temp("roundtrip");
        let (a, b) = (dir.join("a"), dir.join("b"));
        fs::create_dir_all(&a).unwrap();
        let mut recents = Recents::default();
        recents.touch(&a, 1);
        recents.touch(&b, 2);
        recents.touch(&a, 3);
        recents.clone_parent = Some(dir.clone());
        save_to(&dir, &recents).unwrap();
        let loaded = load_from(&dir);
        assert_eq!(loaded, recents);
        assert_eq!(loaded.projects[0].path, a);
        assert_eq!(loaded.projects[0].opened, 3);
        assert_eq!(loaded.projects[0].name, "a");
        assert_eq!(loaded.projects.len(), 2);
        // b does not exist: it stays in the file but not in the menu.
        assert_eq!(loaded.available().len(), 1);
        assert_eq!(loaded.last_existing(), Some(a));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn recents_keep_a_bounded_list() {
        let mut recents = Recents::default();
        for i in 0..30 {
            recents.touch(Path::new(&format!("C:\\p{i}")), i);
        }
        assert_eq!(recents.projects.len(), LIMIT);
        assert_eq!(recents.projects[0].path, Path::new("C:\\p29"));
    }

    #[test]
    fn last_workspace_file_seeds_the_list_once() {
        let dir = temp("migrate");
        let project = dir.join("project");
        fs::create_dir_all(&project).unwrap();
        fs::write(
            dir.join("last-workspace.txt"),
            project.to_string_lossy().as_bytes(),
        )
        .unwrap();
        let recents = load_from(&dir);
        assert_eq!(recents.projects.len(), 1);
        assert_eq!(recents.projects[0].path, project);
        assert!(recents.projects[0].opened > 0);
        assert!(dir.join("projects.json").exists());
        // Once projects.json exists the legacy file is no longer read.
        fs::write(dir.join("last-workspace.txt"), "C:\\elsewhere").unwrap();
        assert_eq!(load_from(&dir).projects[0].path, project);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_damaged_list_reads_as_empty() {
        let dir = temp("damaged");
        fs::write(dir.join("projects.json"), "{not json").unwrap();
        assert_eq!(load_from(&dir), Recents::default());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn shorthand_and_urls_parse_to_the_right_tool() {
        let gh = |o: &str, r: &str| Source::GitHub {
            owner: o.into(),
            repo: r.into(),
        };
        assert_eq!(
            parse_source("  anomalyco/rune "),
            Ok(gh("anomalyco", "rune"))
        );
        assert_eq!(parse_source("owner/repo.git"), Ok(gh("owner", "repo")));
        assert_eq!(
            parse_source("my.org/my_repo-2"),
            Ok(gh("my.org", "my_repo-2"))
        );
        for url in [
            "https://github.com/owner/repo",
            "https://github.com/owner/repo.git",
            "git@github.com:owner/repo.git",
            "ssh://git@host:2222/team/repo.git",
            "https://gitlab.com/group/sub/repo/",
            "file:///C:/repos/thing.git",
        ] {
            assert_eq!(parse_source(url), Ok(Source::Url(url.into())), "{url}");
        }
        for bad in [
            "",
            "   ",
            "owner",
            "owner/repo/extra",
            "-c core.sshCommand=x",
            "--upload-pack=x",
            "https://github.com",
            "https://github.com/",
            "ftp://host/repo",
            "C:\\repos\\thing",
            "C:/repos/thing",
            "owner /repo",
            "../repo",
        ] {
            assert!(parse_source(bad).is_err(), "{bad:?} should be refused");
        }
    }

    #[test]
    fn folder_names_follow_git_clone() {
        let name = |input: &str| parse_source(input).unwrap().folder_name();
        assert_eq!(name("owner/repo").unwrap(), "repo");
        assert_eq!(name("https://github.com/owner/repo.git").unwrap(), "repo");
        assert_eq!(name("https://github.com/owner/repo/").unwrap(), "repo");
        assert_eq!(
            name("https://host/owner/repo.git?x=1#frag").unwrap(),
            "repo"
        );
        assert_eq!(name("git@github.com:owner/repo.git").unwrap(), "repo");
        assert_eq!(name("git@host:repo").unwrap(), "repo");
        assert!(name("https://host/owner/..").is_err());
        assert!(name("https://host/owner/a%3F|b").is_err());
    }

    #[test]
    fn credentials_never_show_in_messages() {
        assert_eq!(
            redact("fatal: repository 'https://me:secret@github.com/o/r/' not found"),
            "fatal: repository 'https://github.com/o/r/' not found"
        );
        assert_eq!(redact("https://github.com/o/r"), "https://github.com/o/r");
        assert_eq!(
            Source::Url("https://tok@host/a/b".into()).display(),
            "https://host/a/b"
        );
    }

    #[test]
    fn destinations_refuse_existing_content() {
        let dir = temp("dest");
        assert_eq!(destination(&dir, "fresh").unwrap(), dir.join("fresh"));
        fs::create_dir_all(dir.join("empty")).unwrap();
        assert_eq!(destination(&dir, "empty").unwrap(), dir.join("empty"));
        fs::create_dir_all(dir.join("full")).unwrap();
        fs::write(dir.join("full").join("x.txt"), "x").unwrap();
        let error = destination(&dir, "full").unwrap_err();
        assert!(error.contains("not empty"), "{error}");
        fs::write(dir.join("file"), "x").unwrap();
        assert!(
            destination(&dir, "file")
                .unwrap_err()
                .contains("not a folder")
        );
        assert!(
            destination(&dir.join("missing"), "x")
                .unwrap_err()
                .contains("does not exist")
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    fn runtime() -> rune::Runtime<State> {
        rune::Runtime::new(
            State::new(".".into(), false),
            crate::ui::view,
            (1280.0, 720.0),
            rune::Env::new(false, 1.0),
        )
    }
    fn press(rt: &mut rune::Runtime<State>, key: &str) {
        let (x, y) = rt
            .find(key)
            .unwrap_or_else(|| panic!("{key} is not on screen"));
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.0);
    }

    #[test]
    fn switcher_opens_the_menu_and_the_clone_sheet() {
        let mut rt = runtime();
        rt.frame(0.0);
        assert!(
            rt.find("open-workspace").is_none(),
            "the old footer button is gone"
        );
        press(&mut rt, "project-switcher");
        assert!(rt.state.projects.menu.is_some());
        rt.frame(1.0 / 60.0);
        assert!(rt.find("project-open-folder").is_some());
        press(&mut rt, "project-clone");
        assert!(rt.state.projects.menu.is_none());
        assert!(rt.state.projects.clone_open);
        rt.frame(1.0 / 60.0);
        press(&mut rt, "clone-start");
        let error = rt.state.projects.clone_error.clone().unwrap();
        assert!(error.contains("owner/repo"), "{error}");
        assert!(!rt.state.projects.cloning());
        press(&mut rt, "clone-cancel");
        assert!(!rt.state.projects.clone_open);
    }

    #[test]
    fn adding_a_project_keeps_busy_threads_and_unsent_drafts() {
        let mut state = State::new(".".into(), false);
        state.current_mut().title = "Port the parser".into();
        state.current_mut().status = crate::model::Status::Running;
        state.current_mut().draft.replace_text("half a thought");
        let id = state.add_project(std::env::temp_dir()).unwrap();
        assert_eq!(state.projects.entries.len(), 2);
        assert_eq!(state.current().project, Some(id));
        assert_eq!(state.threads[0].status, crate::model::Status::Running);
        assert_eq!(state.threads[0].draft.text, "half a thought");
        state.remove_project(id);
        assert!(state.project(Some(id)).unwrap().hidden);
        assert!(std::env::temp_dir().is_dir());
        assert!(state.threads.iter().any(|t| t.project == Some(id)));
        assert_eq!(state.add_project(std::env::temp_dir()).unwrap(), id);
        assert!(!state.project(Some(id)).unwrap().hidden);
    }

    /// Clones a small public repository over the network with both tools. Run it on demand:
    /// `cargo test real_clone_of_a_public_repository -- --ignored`.
    #[test]
    #[ignore]
    fn real_clone_of_a_public_repository() {
        let dir = temp("network");
        for (index, input) in [
            "octocat/Hello-World",
            "https://github.com/octocat/Spoon-Knife",
        ]
        .into_iter()
        .enumerate()
        {
            let source = parse_source(input).unwrap();
            let target = destination(&dir, &source.folder_name().unwrap()).unwrap();
            let job = CloneJob::start(&source, dir.clone(), target.clone(), Arc::new(|| {}));
            let result = loop {
                match job.events.recv().unwrap() {
                    CloneEvent::Done(result) => break result,
                    CloneEvent::Progress(_) => {}
                }
            };
            assert_eq!(result, Ok(()), "clone {index} of {input}");
            assert!(target.join(".git").is_dir());
            assert!(destination(&dir, &source.folder_name().unwrap()).is_err());
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn clone_commands_use_gh_for_shorthand_and_never_prompt() {
        let target = Path::new(r"D:\src\repo");
        let gh = clone_command(&parse_source("o/repo").unwrap(), target);
        assert_eq!(gh.get_program(), "gh");
        let args: Vec<_> = gh
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            [
                "repo",
                "clone",
                "o/repo",
                r"D:\src\repo",
                "--",
                "--progress"
            ]
        );
        let git = clone_command(&parse_source("https://host/o/repo").unwrap(), target);
        assert_eq!(git.get_program(), "git");
        let args: Vec<_> = git
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            [
                "clone",
                "--progress",
                "--",
                "https://host/o/repo",
                r"D:\src\repo"
            ]
        );
        let envs: Vec<_> = git.get_envs().collect();
        assert!(envs.contains(&("GIT_TERMINAL_PROMPT".as_ref(), Some("0".as_ref()))));
    }
}
