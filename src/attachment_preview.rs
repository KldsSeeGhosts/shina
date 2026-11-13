//! Port of KSG ComposerFilePreviewDialog, ExpandedImageDialog and MediaVideoPlayer.
use super::*;
use std::io::Read;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Image,
    Video,
    Audio,
    Pdf,
    Html,
    Markdown,
    Text,
    Sqlite,
    Archive,
    Font,
    Notebook,
    Unsupported,
}

fn kind(name: &str, mime: &str) -> Kind {
    let mime = mime
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let extension = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    let generic = matches!(
        mime.as_str(),
        "" | "application/octet-stream"
            | "binary/octet-stream"
            | "application/unknown"
            | "video/mp2t"
            | "video/vnd.dlna.mpeg-tts"
            | "text/plain"
    );
    if mime == "application/x-ipynb+json"
        || (generic || mime == "application/json") && extension == "ipynb"
    {
        return Kind::Notebook;
    }
    if matches!(
        mime.as_str(),
        "application/zip"
            | "application/x-zip-compressed"
            | "application/java-archive"
            | "application/x-java-archive"
            | "application/x-tar"
            | "application/gzip"
            | "application/x-gzip"
            | "application/x-compressed-tar"
    ) || generic && matches!(extension.as_str(), "zip" | "jar" | "tar" | "gz" | "tgz")
    {
        return Kind::Archive;
    }
    if matches!(
        mime.as_str(),
        "application/vnd.sqlite3" | "application/x-sqlite3" | "application/x-sqlite"
    ) || generic && matches!(extension.as_str(), "sqlite" | "sqlite3" | "db")
    {
        return Kind::Sqlite;
    }
    if mime.starts_with("font/")
        || generic && matches!(extension.as_str(), "ttf" | "otf" | "woff" | "woff2")
        || extension == "otf" && mime == "application/vnd.oasis.opendocument.formula-template"
    {
        return Kind::Font;
    }
    if mime == "application/pdf" || generic && extension == "pdf" {
        return Kind::Pdf;
    }
    if mime == "text/html" || generic && matches!(extension.as_str(), "html" | "htm") {
        return Kind::Html;
    }
    if matches!(mime.as_str(), "text/markdown" | "text/x-markdown")
        || generic
            && matches!(
                extension.as_str(),
                "md" | "markdown" | "mdown" | "mkd" | "mdx"
            )
    {
        return Kind::Markdown;
    }
    if mime.starts_with("image/")
        || generic
            && matches!(
                extension.as_str(),
                "avif"
                    | "bmp"
                    | "gif"
                    | "ico"
                    | "heic"
                    | "heif"
                    | "jpeg"
                    | "jpg"
                    | "png"
                    | "svg"
                    | "tif"
                    | "tiff"
                    | "webp"
            )
    {
        return Kind::Image;
    }
    if mime.starts_with("video/") && !generic
        || generic && crate::attachment_intake::native_mime(name).starts_with("video/")
    {
        return Kind::Video;
    }
    if mime.starts_with("audio/")
        || generic
            && matches!(
                extension.as_str(),
                "wav"
                    | "mp3"
                    | "m4a"
                    | "aac"
                    | "ogg"
                    | "oga"
                    | "flac"
                    | "aiff"
                    | "opus"
                    | "webm-audio"
                    | "weba"
            )
    {
        return Kind::Audio;
    }
    static TEXT_EXTENSION: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"^(txt|log|json|jsonc|jsonl|ndjson|yaml|yml|toml|ini|conf|config|env|csv|tsv|xml|css|scss|sass|less|js|jsx|mjs|cjs|ts|tsx|mts|cts|py|pyi|rb|go|rs|swift|kt|kts|java|c|h|cc|cpp|hpp|cs|php|sh|bash|zsh|fish|sql|graphql|gql|vue|svelte|r|lua|ex|exs|erl|hs|clj|dart|diff|patch|lock|properties|gradle)$").unwrap()
    });
    static TEXT_BASENAME: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?i)^(dockerfile|makefile|gemfile|rakefile|license|readme|\.gitignore|\.gitattributes|\.editorconfig|\.env)(\.|$)").unwrap()
    });
    static TEXT_MIME: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"^application/(json|.*\+json|xml|.*\+xml|javascript|x-javascript|yaml|x-yaml|toml|sql)$").unwrap()
    });
    if mime.starts_with("text/")
        || TEXT_MIME.is_match(&mime)
        || generic
            && (TEXT_EXTENSION.is_match(&extension)
                || TEXT_BASENAME.is_match(name.rsplit(['/', '\\']).next().unwrap_or("")))
    {
        return Kind::Text;
    }
    Kind::Unsupported
}

pub(crate) struct Preview {
    machine: String,
    thread: usize,
    items: Vec<Attachment>,
    index: usize,
    draft: bool,
    player: Option<rune::Player>,
    text: String,
    error: Option<String>,
    truncated: bool,
    rendered: bool,
    wrap: bool,
    zoom: f32,
    image_scroll: Arc<std::sync::Mutex<rune::ScrollPos>>,
    image_drag: Option<[f32; 2]>,
    image_viewport_size: [f32; 2],
    suppress_image_click: bool,
    document: Option<Arc<attachment_text_preview::Document>>,
    cancel: Arc<AtomicBool>,
    database: Option<Arc<attachment_sqlite_preview::Database>>,
    database_page: Option<attachment_sqlite_preview::Page>,
    database_table: usize,
    database_page_index: usize,
    database_picker: bool,
    database_loading: bool,
    archive: Option<Arc<attachment_archive_preview::Archive>>,
    archive_loading: bool,
    archive_folder: String,
    archive_entry: Option<(String, Result<ArchiveContent, String>)>,
    notebook: Option<Arc<attachment_notebook_preview::Notebook>>,
    notebook_loading: bool,
    notebook_visible: usize,
}

enum ArchiveContent {
    Image(rune::Image),
    Text { text: String, truncated: bool },
}

impl Drop for Preview {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}

impl Preview {
    #[cfg(test)]
    pub(super) fn zoom(&self) -> f32 {
        self.zoom
    }
    #[cfg(test)]
    pub(super) fn scroll_offset(&self) -> [f32; 2] {
        self.scroll().offset
    }
    fn scroll(&self) -> rune::ScrollPos {
        *self.image_scroll.lock().unwrap_or_else(|e| e.into_inner())
    }
    fn change_zoom(&mut self, next: f32, point: Option<[f32; 2]>, tx: &mut Tx) {
        let next = clamped_zoom(next);
        let scroll = self.scroll();
        let point = point.unwrap_or([scroll.viewport[0] / 2.0, scroll.viewport[1] / 2.0]);
        let offset = zoom_offset(self.zoom, next, scroll.offset, point);
        self.zoom = next;
        tx.scroll_to_instant("attachment-preview-image-viewport", offset[0], offset[1]);
    }
    #[cfg(test)]
    pub(super) fn player(&self) -> Option<&rune::Player> {
        self.player.as_ref()
    }
    fn selected(&self) -> &Attachment {
        &self.items[self.index]
    }
    fn load(&mut self, wake: rune::Waker) {
        self.cancel.store(true, Ordering::Release);
        self.cancel = Arc::new(AtomicBool::new(false));
        self.document = None;
        self.database = None;
        self.database_page = None;
        self.archive = None;
        self.archive_loading = false;
        self.archive_folder.clear();
        self.archive_entry = None;
        self.notebook = None;
        self.notebook_loading = false;
        self.notebook_visible = 20;
        self.player = None;
        self.text.clear();
        self.error = None;
        self.truncated = false;
        self.zoom = 1.0;
        *self.image_scroll.lock().unwrap_or_else(|e| e.into_inner()) = Default::default();
        self.image_drag = None;
        self.image_viewport_size = [0.0; 2];
        self.suppress_image_click = false;
        let attachment = self.selected().clone();
        let Some(path) = crate::attachments::path_for(&attachment).filter(|p| p.is_file()) else {
            self.error = Some("The attachment is unavailable.".into());
            return;
        };
        match kind(&attachment.name, &attachment.mime) {
            Kind::Video | Kind::Audio => {
                let player = if kind(&attachment.name, &attachment.mime) == Kind::Audio {
                    rune::Player::audio(path)
                } else {
                    rune::Player::file(path)
                };
                player.set_muted(false);
                player.set_waker(wake);
                if kind(&attachment.name, &attachment.mime) == Kind::Video {
                    player.play();
                }
                self.player = Some(player);
            }
            Kind::Text | Kind::Markdown | Kind::Html => {
                let delimiter = if self.rendered {
                    attachment_text_preview::delimiter(&attachment.name, &attachment.mime)
                } else {
                    None
                };
                let streaming = kind(&attachment.name, &attachment.mime) == Kind::Text
                    && (attachment.bytes > 1024 * 1024 || delimiter.is_some());
                if streaming {
                    let result = (|| -> Result<_, String> {
                        let mut bytes = Vec::new();
                        std::fs::File::open(&path)
                            .map_err(|e| e.to_string())?
                            .take(attachment_text_preview::FIRST_SCREEN as u64)
                            .read_to_end(&mut bytes)
                            .map_err(|e| e.to_string())?;
                        let complete = bytes.len() as u64 == attachment.bytes;
                        if rune::is_headless() && !complete {
                            attachment_text_preview::Document::load(&path, delimiter, &self.cancel)
                        } else {
                            attachment_text_preview::Document::index(bytes, delimiter, complete)
                        }
                    })();
                    match result {
                        Ok(document) => self.document = Some(Arc::new(document)),
                        Err(error) => self.error = Some(error),
                    }
                    return;
                }
                let result = (|| {
                    let mut bytes = Vec::new();
                    std::fs::File::open(path)?
                        .take(1024 * 1024 + 1)
                        .read_to_end(&mut bytes)?;
                    self.truncated = bytes.len() > 1024 * 1024;
                    bytes.truncate(1024 * 1024);
                    attachment_text_preview::decode_small(&bytes, self.truncated)
                        .map_err(std::io::Error::other)
                })();
                match result {
                    Ok(text) => self.text = text,
                    Err(e) => self.error = Some(e.to_string()),
                }
            }
            Kind::Font if attachment.bytes > 10 * 1024 * 1024 => {
                self.error = Some("Font previews are limited to 10 MB.".into());
            }
            _ => {}
        }
    }
}

impl State {
    pub(crate) fn expire_attachment_preview(&mut self) {
        if self
            .attachment_preview
            .as_ref()
            .is_some_and(|p| p.machine != self.machines.selected || p.thread != self.current().id)
        {
            self.attachment_preview = None;
        }
    }
    pub(crate) fn pause_attachment_preview(&self) {
        if let Some(player) = self
            .attachment_preview
            .as_ref()
            .and_then(|p| p.player.as_ref())
        {
            player.pause();
        }
    }
    pub(crate) fn preview_attachments(&mut self, items: Vec<Attachment>, id: &str, draft: bool) {
        let Some(selected) = items.iter().find(|a| a.id == id).cloned() else {
            return;
        };
        if selected.needs_reattach() {
            return;
        }
        // ExpandedImagePreview builds a gallery only for images. Files and video stand alone.
        let items: Vec<_> = if selected.is_image() {
            items.into_iter().filter(Attachment::is_image).collect()
        } else {
            vec![selected]
        };
        let index = items.iter().position(|a| a.id == id).unwrap_or(0);
        let mut preview = Preview {
            machine: self.machines.selected.clone(),
            thread: self.current().id,
            items,
            index,
            draft,
            player: None,
            text: String::new(),
            error: None,
            truncated: false,
            rendered: true,
            wrap: true,
            zoom: 1.0,
            image_scroll: Arc::new(std::sync::Mutex::new(Default::default())),
            image_drag: None,
            image_viewport_size: [0.0; 2],
            suppress_image_click: false,
            document: None,
            cancel: Arc::new(AtomicBool::new(false)),
            database: None,
            database_page: None,
            database_table: 0,
            database_page_index: 0,
            database_picker: false,
            database_loading: false,
            archive: None,
            archive_loading: false,
            archive_folder: String::new(),
            archive_entry: None,
            notebook: None,
            notebook_loading: false,
            notebook_visible: 20,
        };
        preview.load(self.wake.clone());
        let fetch = preview.error.is_some() && preview.selected().uploaded.is_some();
        let file = preview.selected().clone();
        let loading = preview.cancel.clone();
        let origin_thread = preview.thread;
        if (matches!(
            kind(&preview.selected().name, &preview.selected().mime),
            Kind::Pdf | Kind::Html | Kind::Font
        ) || preview.selected().mime == "image/avif")
            && !rune::is_headless()
            && preview.error.is_none()
        {
            self.html_windows.open_attachment_async(
                preview.selected().clone(),
                self.dark,
                self.wake.clone(),
            );
        }
        self.attachment_preview = Some(preview);
        self.load_attachment_text_document();
        if kind(&file.name, &file.mime) == Kind::Sqlite {
            self.load_attachment_database(0, 0);
        }
        if kind(&file.name, &file.mime) == Kind::Archive {
            self.load_attachment_archive();
        }
        if kind(&file.name, &file.mime) == Kind::Notebook {
            self.load_attachment_notebook();
        }
        if fetch
            && let (Some(link), Some(handle)) = (
                self.attachment_preview_link(),
                self.background.handle.clone(),
            )
        {
            let machine = self.machines.selected.clone();
            let profile = self.data_dir.clone();
            std::thread::spawn(move || {
                let result = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|e| e.to_string())
                    .and_then(|runtime| {
                        runtime.block_on(crate::attachment_uploads::download_staged(
                            &link, &profile, &file,
                        ))
                    });
                handle.update(move |state, _| {
                    if state.machines.selected != machine || state.current().id != origin_thread {
                        return;
                    }
                    let Some(preview) = &mut state.attachment_preview else {
                        return;
                    };
                    if preview.selected().id != file.id || !Arc::ptr_eq(&preview.cancel, &loading) {
                        return;
                    }
                    match result {
                        Ok(fetched) => {
                            preview.items[preview.index] = fetched.clone();
                            preview.load(state.wake.clone());
                            if let Some(draft) = state
                                .current_mut()
                                .draft_images
                                .iter_mut()
                                .find(|a| a.id == file.id)
                            {
                                *draft = fetched;
                            }
                            state.checkpoint();
                            // Loading downloaded bytes can start an indexed text/database
                            // preview or an isolated document, exactly like a local source.
                            state.retry_attachment_preview();
                        }
                        Err(error) => preview.error = Some(error),
                    }
                });
            });
        }
    }

    fn navigate_attachment_preview(&mut self, direction: isize) {
        if let Some(preview) = &mut self.attachment_preview {
            preview.index = (preview.index as isize + direction)
                .rem_euclid(preview.items.len() as isize) as usize;
        }
        self.retry_attachment_preview();
    }

    fn retry_attachment_preview(&mut self) {
        if let Some(preview) = &self.attachment_preview {
            let (items, id, draft) = (
                preview.items.clone(),
                preview.selected().id.clone(),
                preview.draft,
            );
            self.preview_attachments(items, &id, draft);
        }
    }

    fn toggle_attachment_table(&mut self) {
        if let Some(preview) = &mut self.attachment_preview {
            preview.rendered = !preview.rendered;
            preview.load(self.wake.clone());
        }
        self.load_attachment_text_document();
    }

    fn load_attachment_text_document(&mut self) {
        let Some(preview) = &self.attachment_preview else {
            return;
        };
        if !preview.document.as_ref().is_some_and(|d| !d.complete) {
            return;
        }
        let Some(handle) = self.background.handle.clone() else {
            return;
        };
        let file = preview.selected().clone();
        let Some(path) = crate::attachments::path_for(&file) else {
            return;
        };
        let cancel = preview.cancel.clone();
        let delimiter = if preview.rendered {
            attachment_text_preview::delimiter(&file.name, &file.mime)
        } else {
            None
        };
        std::thread::spawn(move || {
            let result = attachment_text_preview::Document::load(&path, delimiter, &cancel);
            if cancel.load(Ordering::Acquire) {
                return;
            }
            handle.update(move |state, _| {
                if let Some(preview) = &mut state.attachment_preview
                    && Arc::ptr_eq(&preview.cancel, &cancel)
                {
                    match result {
                        Ok(document) => preview.document = Some(Arc::new(document)),
                        Err(error) => preview.error = Some(error),
                    }
                }
            });
        });
    }

    fn load_attachment_database(&mut self, table: usize, page: usize) {
        let Some(preview) = &mut self.attachment_preview else {
            return;
        };
        if preview.database_loading {
            return;
        }
        preview.database_loading = true;
        preview.database_picker = false;
        let database = preview.database.clone();
        let file = preview.selected().clone();
        let cancel = preview.cancel.clone();
        let work = move || -> Result<_, String> {
            let database = if let Some(database) = database {
                database
            } else {
                Arc::new(attachment_sqlite_preview::Database::open(&file)?)
            };
            let rows = if let Some(table) = database.tables.get(table) {
                Some(database.page(&table.name, page)?)
            } else {
                None
            };
            Ok((database, rows))
        };
        let apply = move |state: &mut State,
                          result: Result<
            (
                Arc<attachment_sqlite_preview::Database>,
                Option<attachment_sqlite_preview::Page>,
            ),
            String,
        >| {
            if let Some(preview) = &mut state.attachment_preview
                && Arc::ptr_eq(&cancel, &preview.cancel)
            {
                preview.database_loading = false;
                match result {
                    Ok((database, rows)) => {
                        preview.database = Some(database);
                        preview.database_page = rows;
                        preview.database_table = table;
                        preview.database_page_index = page;
                    }
                    Err(error) => preview.error = Some(error),
                }
            }
        };
        if let Some(handle) = self.background.handle.clone() {
            std::thread::spawn(move || {
                let result = work();
                handle.update(move |s, _| apply(s, result));
            });
        } else {
            apply(self, work());
        }
    }

    fn load_attachment_archive(&mut self) {
        let Some(preview) = &mut self.attachment_preview else {
            return;
        };
        if preview.archive_loading || preview.error.is_some() {
            return;
        }
        preview.archive_loading = true;
        let file = preview.selected().clone();
        let cancel = preview.cancel.clone();
        let work = move || {
            let path =
                crate::attachments::path_for(&file).ok_or("The attachment is unavailable.")?;
            let mut bytes = Vec::new();
            std::fs::File::open(path)
                .map_err(|e| e.to_string())?
                .take(crate::attachments::MAX_FILE_BYTES + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            attachment_archive_preview::Archive::parse(&file.name, &bytes).map(Arc::new)
        };
        let apply = move |state: &mut State, result| {
            if let Some(preview) = &mut state.attachment_preview
                && Arc::ptr_eq(&preview.cancel, &cancel)
            {
                preview.archive_loading = false;
                match result {
                    Ok(archive) => preview.archive = Some(archive),
                    Err(error) => preview.error = Some(error),
                }
            }
        };
        if let Some(handle) = self.background.handle.clone() {
            std::thread::spawn(move || {
                let result = work();
                handle.update(move |state, _| apply(state, result));
            });
        } else {
            apply(self, work());
        }
    }

    fn load_attachment_notebook(&mut self) {
        let Some(preview) = &mut self.attachment_preview else {
            return;
        };
        if preview.notebook_loading || preview.error.is_some() {
            return;
        }
        preview.notebook_loading = true;
        let file = preview.selected().clone();
        let cancel = preview.cancel.clone();
        let work = move || {
            let path =
                crate::attachments::path_for(&file).ok_or("The attachment is unavailable.")?;
            let mut bytes = Vec::new();
            std::fs::File::open(path)
                .map_err(|e| e.to_string())?
                .take(attachment_notebook_preview::MAX_BYTES as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            attachment_notebook_preview::parse(&bytes).map(Arc::new)
        };
        let apply = move |state: &mut State, result| {
            if let Some(preview) = &mut state.attachment_preview
                && Arc::ptr_eq(&preview.cancel, &cancel)
            {
                preview.notebook_loading = false;
                match result {
                    Ok(notebook) => preview.notebook = Some(notebook),
                    Err(error) => preview.error = Some(error),
                }
            }
        };
        if let Some(handle) = self.background.handle.clone() {
            std::thread::spawn(move || {
                let result = work();
                handle.update(move |state, _| apply(state, result));
            });
        } else {
            apply(self, work());
        }
    }

    fn open_archive_entry(&mut self, path: &str) {
        let Some(preview) = &mut self.attachment_preview else {
            return;
        };
        let Some(archive) = &preview.archive else {
            return;
        };
        let result = archive.entry(path).and_then(|bytes| {
            let mime = crate::attachment_intake::native_mime(path);
            match kind(path, mime) {
                Kind::Image => Ok(ArchiveContent::Image(self.attachment_cache.load_mime(
                    Arc::<[u8]>::from(bytes),
                    mime,
                    self.viewport_width - 64.0,
                    2.0,
                ))),
                Kind::Text | Kind::Markdown | Kind::Html => {
                    let truncated = bytes.len() > 1024 * 1024;
                    attachment_text_preview::decode_small(
                        &bytes[..bytes.len().min(1024 * 1024)],
                        truncated,
                    )
                    .map(|text| ArchiveContent::Text { text, truncated })
                }
                _ => Err("Only UTF-8 text and images can be opened inside an archive.".into()),
            }
        });
        preview.archive_entry = Some((path.into(), result));
    }
}

// KSG ZoomableImage.tsx: maximum zoom and pointer-anchored scroll arithmetic.
fn clamped_zoom(zoom: f32) -> f32 {
    zoom.clamp(1.0, 8.0)
}

fn zoom_offset(old: f32, new: f32, offset: [f32; 2], point: [f32; 2]) -> [f32; 2] {
    std::array::from_fn(|i| ((offset[i] + point[i]) / old * new - point[i]).max(0.0))
}

/// Ported from KSG NotebookPreview: the cell count and language strip, the
/// saved-outputs notice, and twenty cells at a time in a scroll region.
fn notebook_view(s: &State, p: Palette, width: f32) -> El {
    let preview = s.attachment_preview.as_ref().unwrap();
    let Some(notebook) = &preview.notebook else {
        return col()
            .w_full()
            .min_w(0.0)
            .h(0.0)
            .grow(1.0)
            .min_h(0.0)
            .child(label("Loading file preview…", SMALL, p.muted));
    };
    let attachment = preview.selected();
    let size = if attachment.bytes == 0 {
        "0 bytes".to_owned()
    } else if attachment.bytes.div_ceil(1024) >= 1024 {
        format!("{:.1} MB", attachment.bytes as f64 / 1048576.0)
    } else {
        format!("{} KB", attachment.bytes.div_ceil(1024))
    };
    let mut cells = col().w_full().min_w(0.0).px(20.0).py(24.0).gap(20.0);
    for (index, cell) in notebook
        .cells
        .iter()
        .take(preview.notebook_visible)
        .enumerate()
    {
        cells = cells.child(notebook_cell(s, p, index, cell, &notebook.language, width));
    }
    if notebook.cells.is_empty() {
        cells = cells.child(label("This notebook is empty.", SMALL, p.muted));
    }
    if preview.notebook_visible < notebook.cells.len() {
        cells = cells.child(framed(
            row()
                .key("notebook-more")
                .role("button")
                .cursor(rune::Cursor::Pointer)
                .label("Show 20 more cells")
                .focusable(true)
                .center_y()
                .h(32.0)
                .px(14.0)
                .radius(4.0)
                .fill(p.card)
                .hover(p.selected)
                .press(p.selected)
                .on_press(|s: &mut State| {
                    if let Some(preview) = &mut s.attachment_preview {
                        preview.notebook_visible += 20;
                    }
                })
                .child(label(
                    "Show 20 more cells",
                    TextStyle::new(13.0, 600.0),
                    p.ink,
                )),
            p,
        ));
    }
    col()
        .w_full()
        .min_w(0.0)
        .h(0.0)
        .grow(1.0)
        .min_h(0.0)
        .gap(8.0)
        .child(pixel_text(
            format!(
                "{} cells · {} · {}",
                notebook.cells.len(),
                notebook.language,
                size
            ),
            p.muted,
        ))
        .child(label(
            "Saved outputs only. Code is never executed.",
            SMALL,
            p.muted,
        ))
        .child(
            col()
                .key("attachment-preview-notebook")
                .w_full()
                .min_w(0.0)
                .h(0.0)
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(cells),
        )
}

/// One KSG NotebookPreview section: the kind and position header, then the
/// source and each saved output.
fn notebook_cell(
    s: &State,
    p: Palette,
    index: usize,
    cell: &attachment_notebook_preview::NotebookCell,
    language: &str,
    width: f32,
) -> El {
    use attachment_notebook_preview::{CellKind, NotebookOutput};
    let mut section = col()
        .key(format!("notebook-cell-{index}"))
        .label(format!("Cell {}", index + 1))
        .w_full()
        .min_w(0.0)
        .radius(6.0)
        .clip_rounded(6.0)
        .border(1.0, p.line)
        .child(
            el().w_full()
                .min_w(0.0)
                .px(12.0)
                .py(6.0)
                .fill(p.code_bg)
                .child(pixel_label(
                    &format!(
                        "{} · {}",
                        if cell.kind == CellKind::Code {
                            language
                        } else {
                            cell.kind.name()
                        },
                        index + 1
                    ),
                    p.muted,
                )),
        );
    match cell.kind {
        CellKind::Markdown => {
            section = section.child(el().w_full().min_w(0.0).px(16.0).py(12.0).child(
                markdown_ui::render_with_find(
                    &cell.source,
                    p,
                    s.dark,
                    width > 760.0,
                    false,
                    false,
                    &format!("notebook-md-{index}"),
                    None,
                ),
            ));
        }
        _ if !cell.source.trim().is_empty() => {
            section = section.child(notebook_source(cell, p));
        }
        _ => {}
    }
    for output in &cell.outputs {
        section = section.child(rule(p)).child(match output {
            NotebookOutput::Text { text, danger } => {
                el().w_full().min_w(0.0).px(16.0).py(12.0).child(
                    label(
                        text,
                        TextStyle::new(12.0, 400.0).family(rune::Family::SystemMono),
                        if *danger { p.danger } else { p.ink },
                    )
                    .wrap_anywhere()
                    .selectable(true),
                )
            }
            NotebookOutput::Html => el().w_full().min_w(0.0).px(16.0).py(12.0).child(label(
                "HTML output is not shown",
                SMALL,
                p.muted,
            )),
            NotebookOutput::Image { mime, bytes, size } => {
                let max_width = (width - 104.0).max(120.0);
                let handle = s
                    .attachment_cache
                    .load_mime(bytes.clone(), mime, max_width, 2.0);
                // Headless fixtures have no window waker to rebuild after decode.
                if rune::is_headless() {
                    let _ = handle.wait();
                }
                if handle.error().is_some() {
                    el().w_full().min_w(0.0).px(16.0).py(12.0).child(label(
                        "Unable to load image.",
                        SMALL,
                        p.muted,
                    ))
                } else {
                    let [natural_width, natural_height] = handle.size().unwrap_or(*size);
                    let fit = (max_width / natural_width.max(1) as f32)
                        .min(384.0 / natural_height.max(1) as f32)
                        .min(1.0);
                    el().px(16.0).py(16.0).child(
                        el().w(natural_width as f32 * fit)
                            .h(natural_height as f32 * fit)
                            .pixels_fit(handle.pixels(), rune::SurfaceFit::Fit),
                    )
                }
            }
        });
    }
    section
}

/// KSG's read-only cell source block: mono source on a `message_bg` block, with
/// Jupyter's execution count in the gutter and the editor's height bound.
fn notebook_source(cell: &attachment_notebook_preview::NotebookCell, p: Palette) -> El {
    let mut body = row().w_full().min_w(0.0).px(10.0).py(8.0).gap(10.0);
    if cell.kind == attachment_notebook_preview::CellKind::Code {
        body = body.child(pixel_text(
            cell.execution_count
                .map(|count| format!("[{count}]"))
                .unwrap_or_else(|| "[ ]".into()),
            p.faint,
        ));
    }
    body = body.child(
        el().w(0.0)
            .grow(1.0)
            .min_w(0.0)
            .max_h(360.0)
            .scroll_xy()
            .child(
                label(
                    &cell.source,
                    TextStyle::new(12.0, 400.0).family(rune::Family::SystemMono),
                    p.ink,
                )
                .selectable(true),
            ),
    );
    el().w_full().min_w(0.0).px(16.0).py(8.0).child(
        el().w_full()
            .min_w(0.0)
            .radius(4.0)
            .clip_rounded(4.0)
            .fill(p.message_bg)
            .child(body),
    )
}

/// Ported from KSG BinaryViewer's archive branch. Archive paths never become host paths.
fn archive_view(s: &State, p: Palette) -> El {
    let preview = s.attachment_preview.as_ref().unwrap();
    let Some(archive) = &preview.archive else {
        return label("Loading file preview…", SMALL, p.muted);
    };
    let selected = preview.archive_entry.as_ref();
    let mut content = col()
        .key("attachment-preview-archive")
        .w_full()
        .min_w(0.0)
        .h(0.0)
        .grow(1.0)
        .min_h(0.0)
        .gap(8.0)
        .child(
            row()
                .w_full()
                .min_w(0.0)
                .gap(8.0)
                .center_y()
                .child(
                    button(
                        "archive-preview-back",
                        if selected.is_some() {
                            "Back to archive"
                        } else {
                            "Parent folder"
                        },
                        p,
                        |s| {
                            if let Some(preview) = &mut s.attachment_preview {
                                if preview.archive_entry.take().is_none() {
                                    let folder = preview.archive_folder.trim_end_matches('/');
                                    preview.archive_folder = folder
                                        .rsplit_once('/')
                                        .map(|(parent, _)| format!("{parent}/"))
                                        .unwrap_or_default();
                                }
                            }
                        },
                    )
                    .disabled(selected.is_none() && preview.archive_folder.is_empty()),
                )
                .child(
                    label(
                        selected.map(|(path, _)| path.as_str()).unwrap_or(
                            if preview.archive_folder.is_empty() {
                                "Archive contents"
                            } else {
                                &preview.archive_folder
                            },
                        ),
                        SMALL,
                        p.ink,
                    )
                    .truncate()
                    .w(0.0)
                    .grow(1.0)
                    .min_w(0.0),
                )
                .child(pixel_text(
                    format!("{} entries", archive.entries.len()),
                    p.muted,
                )),
        );
    if let Some((_, entry)) = selected {
        match entry {
            Err(error) => content = content.child(label(error, SMALL, p.danger).wrap()),
            Ok(ArchiveContent::Image(image)) => {
                if rune::is_headless() {
                    let _ = image.wait();
                }
                if image.error().is_some() {
                    content = content.child(label("Unable to load image.", SMALL, p.danger));
                } else {
                    content = content.child(
                        el().key("archive-preview-image")
                            .w_full()
                            .h(0.0)
                            .grow(1.0)
                            .pixels_fit(image.pixels(), rune::SurfaceFit::Fit),
                    );
                }
            }
            Ok(ArchiveContent::Text { text, truncated }) => {
                if *truncated {
                    content =
                        content.child(label("Preview limited to the first 1 MB.", SMALL, p.muted));
                }
                content = content.child(
                    col()
                        .key("archive-preview-source")
                        .w_full()
                        .min_w(0.0)
                        .h(0.0)
                        .grow(1.0)
                        .min_h(0.0)
                        .scroll_xy()
                        .child(
                            label(
                                text,
                                TextStyle::new(13.0, 400.0).family(rune::Family::SystemMono),
                                p.ink,
                            )
                            .selectable(true),
                        ),
                );
            }
        }
    } else {
        let mut children =
            std::collections::BTreeMap::<String, (bool, usize, Option<String>)>::new();
        for entry in &archive.entries {
            let Some(rest) = entry.path.strip_prefix(&preview.archive_folder) else {
                continue;
            };
            let rest = rest.trim_end_matches('/');
            if rest.is_empty() {
                continue;
            }
            let component = rest.split('/').next().unwrap();
            let directory = rest.contains('/') || entry.directory;
            let path = format!(
                "{}{component}{}",
                preview.archive_folder,
                if directory { "/" } else { "" }
            );
            let child = children
                .entry(path)
                .or_insert((directory, 0, entry.modified.clone()));
            child.1 = child.1.saturating_add(entry.bytes.len());
        }
        let mut children: Vec<_> = children.into_iter().collect();
        children.sort_by(|a, b| b.1.0.cmp(&a.1.0).then_with(|| a.0.cmp(&b.0)));
        let mut list = col().w_full().min_w(0.0).gap(4.0);
        for (index, (path, (directory, size, modified))) in children.into_iter().enumerate() {
            let name = path
                .trim_end_matches('/')
                .rsplit('/')
                .next()
                .unwrap_or(&path)
                .to_owned();
            let target = path.clone();
            list = list.child(
                el().key(format!("archive-preview-entry-{index}"))
                    .role("button")
                    .label(&name)
                    .focusable(true)
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |s: &mut State| {
                        if directory {
                            if let Some(preview) = &mut s.attachment_preview {
                                preview.archive_folder = target.clone();
                            }
                        } else {
                            s.open_archive_entry(&target);
                        }
                    })
                    .child(
                        row()
                            .w_full()
                            .min_w(0.0)
                            .gap(8.0)
                            .px(12.0)
                            .py(8.0)
                            .center_y()
                            .child(icon(if directory { "folder" } else { "file" }, p.muted))
                            .child(
                                label(name, SMALL, p.ink)
                                    .truncate()
                                    .w(0.0)
                                    .grow(1.0)
                                    .min_w(0.0),
                            )
                            .child(pixel_text(
                                if size.div_ceil(1024) >= 1024 {
                                    format!("{:.1} MB", size as f64 / 1048576.0)
                                } else {
                                    format!("{} KB", size.div_ceil(1024).max(1))
                                },
                                p.muted,
                            ))
                            .child(
                                label(
                                    modified.unwrap_or_default(),
                                    TextStyle::new(11.0, 400.0),
                                    p.faint,
                                )
                                .truncate()
                                .max_w(180.0),
                            ),
                    ),
            );
        }
        content = content.child(
            col()
                .w_full()
                .min_w(0.0)
                .h(0.0)
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(list),
        );
    }
    content
}

pub(super) fn view(s: &State, p: Palette) -> El {
    let preview = s.attachment_preview.as_ref().unwrap();
    if preview.machine != s.machines.selected || preview.thread != s.current().id {
        return el();
    }
    let attachment = preview.selected();
    let attachment_kind = kind(&attachment.name, &attachment.mime);
    let media = matches!(attachment_kind, Kind::Image | Kind::Video);
    let width = if media {
        s.viewport_width * 0.92
    } else {
        896.0_f32.min(s.viewport_width - 32.0)
    };
    let height = (s.viewport_height * 0.85).min(832.0);
    let mut header = row().w_full().min_w(0.0).gap(8.0).center_y().child(
        label(&attachment.name, DIALOG_TITLE, p.ink)
            .truncate()
            .w(0.0)
            .grow(1.0)
            .min_w(0.0),
    );
    if preview.draft && !media {
        header = header.child(pixel_text("Draft", p.muted));
    }
    let file = attachment.clone();
    header = header.child(
        icon_button(
            "download-attachment-preview",
            "Download file",
            "file",
            p,
            |_| {},
        )
        .on_press_tx(move |_: &mut State, tx| {
            let file = file.clone();
            tx.save_file(
                rune::FileDialogOptions {
                    title: "Download file".into(),
                    file_name: Some(file.name.clone()),
                    ..Default::default()
                },
                move |s: &mut State, result, _| {
                    let result = match result {
                        Ok(Some(target)) => crate::attachments::path_for(&file)
                            .ok_or("The attachment is unavailable.".to_owned())
                            .and_then(|source| {
                                std::fs::copy(source, target)
                                    .map(|_| ())
                                    .map_err(|e| e.to_string())
                            }),
                        Ok(None) => return,
                        Err(e) => Err(e.to_string()),
                    };
                    if let Err(error) = result {
                        s.push_toast(
                            crate::model::ToastKind::Error,
                            "Could not save file",
                            Some(error),
                            None,
                            Some(6000),
                        );
                    }
                },
            );
        }),
    );
    if preview.draft {
        let id = attachment.id.clone();
        header = header.child(icon_button(
            "remove-preview-attachment",
            "Remove file",
            "trash",
            p,
            move |s| {
                s.request_remove_attachment(&id);
                s.attachment_preview = None;
            },
        ));
    }
    header = header.child(
        el().key("close-transcript-context-preview")
            .child(icon_button(
                "close-attachment-preview",
                "Close preview",
                "close",
                p,
                |s| {
                    s.attachment_preview = None;
                    s.restore_composer_focus(composer_ui::FocusReason::Explicit);
                },
            )),
    );
    let mut card = col()
        .key("attachment-preview")
        .role("dialog")
        .label(if media {
            "Expanded media preview"
        } else {
            &attachment.name
        })
        .w(width)
        .min_w(0.0)
        .max_w(s.viewport_width - 24.0)
        .h(height)
        .min_h(0.0)
        .padding(16.0)
        .gap(12.0)
        .radius(6.0)
        .fill(p.card)
        .border(2.0, p.frame)
        .box_shadow(hard_shadow(p))
        .on_press(|_: &mut State| {})
        .child(header);
    if let Some(error) = &preview.error {
        card = card
            .child(
                label(error, BODY, p.danger)
                    .key("attachment-preview-error")
                    .wrap(),
            )
            .child(button("retry-attachment-preview", "Retry", p, |s| {
                s.retry_attachment_preview();
            }));
    } else {
        match attachment_kind {
            Kind::Image => {
                let image_scroll = preview.image_scroll.clone();
                let path = crate::attachments::path_for(attachment)
                    .or_else(|| crate::attachments::preview_path_for(attachment));
                let mut image = el()
                    .key("attachment-preview-image")
                    .w((width - 36.0) * preview.zoom)
                    .h((height - 120.0).max(40.0) * preview.zoom);
                if let Some(path) = path {
                    let handle = s.attachment_cache.file(path, width * preview.zoom, 2.0);
                    if rune::is_headless() {
                        let _ = handle.wait();
                    }
                    if handle.error().is_some() {
                        let file = attachment.clone();
                        image = image.child(
                            col()
                                .center()
                                .child(label("Unable to load image.", SMALL, p.muted))
                                .child(button(
                                    "open-native-attachment-preview",
                                    "Preview",
                                    p,
                                    move |s| {
                                        s.html_windows.open_attachment_async(
                                            file.clone(),
                                            s.dark,
                                            s.wake.clone(),
                                        )
                                    },
                                )),
                        );
                    } else {
                        let [natural_width, natural_height] = handle
                            .size()
                            .unwrap_or([attachment.width.max(1), attachment.height.max(1)]);
                        let fit = ((width - 36.0) / natural_width as f32)
                            .min((height - 120.0).max(40.0) / natural_height as f32)
                            .min(1.0);
                        image = image
                            .w(natural_width as f32 * fit * preview.zoom)
                            .h(natural_height as f32 * fit * preview.zoom)
                            .pixels_fit(handle.pixels(), rune::SurfaceFit::Fit);
                    }
                }
                card = card.child(
                    el().key("attachment-preview-image-viewport")
                        .w_full()
                        .h(0.0)
                        .grow(1.0)
                        .min_h(0.0)
                        .scroll_xy()
                        .role("region")
                        .label(format!("{}, zoomable image", attachment.name))
                        .focusable(true)
                        .on_scroll_offset(move |scroll| {
                            *image_scroll.lock().unwrap_or_else(|e| e.into_inner()) = scroll;
                        })
                        .on_layout(|s: &mut State, _| {
                            // KSG refits on viewer/window size changes, not when
                            // zooming changes an inner scrollbar's viewport.
                            let size = [s.viewport_width, s.viewport_height];
                            if let Some(preview) = &mut s.attachment_preview {
                                if preview.image_viewport_size != [0.0; 2]
                                    && preview.image_viewport_size != size
                                {
                                    preview.zoom = 1.0;
                                    preview.image_drag = None;
                                }
                                preview.image_viewport_size = size;
                            }
                        })
                        .on_wheel(|s: &mut State, event, tx| {
                            if !event.mods.ctrl || event.dy == 0.0 {
                                return false;
                            }
                            if let Some(preview) = &mut s.attachment_preview {
                                // Rune's wheel delta has the opposite sign to DOM deltaY.
                                preview.change_zoom(
                                    preview.zoom * (event.dy * 0.01).exp(),
                                    Some([event.x, event.y]),
                                    tx,
                                );
                            }
                            true
                        })
                        .on_magnify(|s: &mut State, event, tx| {
                            if let Some(preview) = &mut s.attachment_preview {
                                preview.change_zoom(
                                    preview.zoom * (1.0 + event.delta),
                                    Some([event.x, event.y]),
                                    tx,
                                );
                            }
                            true
                        })
                        .on_drag(|s: &mut State, drag, tx| {
                            let Some(preview) = &mut s.attachment_preview else {
                                return;
                            };
                            match drag.phase {
                                rune::Phase::Down => {
                                    preview.suppress_image_click = false;
                                    preview.image_drag =
                                        (preview.zoom > 1.0).then(|| preview.scroll().offset);
                                }
                                rune::Phase::Move => {
                                    if let Some(origin) = preview.image_drag {
                                        if drag.dx.hypot(drag.dy) > 4.0 {
                                            preview.suppress_image_click = true;
                                        }
                                        tx.scroll_to_instant(
                                            "attachment-preview-image-viewport",
                                            (origin[0] - drag.dx).max(0.0),
                                            (origin[1] - drag.dy).max(0.0),
                                        );
                                    }
                                }
                                rune::Phase::Up => {
                                    preview.image_drag = None;
                                    if !drag.cancelled()
                                        && !preview.suppress_image_click
                                        && drag.clicks <= 1
                                    {
                                        preview.change_zoom(
                                            if preview.zoom > 1.0 { 1.0 } else { 2.0 },
                                            Some([drag.x, drag.y]),
                                            tx,
                                        );
                                    }
                                }
                            }
                        })
                        .child(image),
                );
                card = card.child(
                    row()
                        .gap(8.0)
                        .child(
                            button("attachment-preview-zoom-out", "−", p, |_| {}).on_press_tx(
                                |s: &mut State, tx| {
                                    if let Some(preview) = &mut s.attachment_preview {
                                        preview.change_zoom(preview.zoom / 1.5, None, tx);
                                    }
                                },
                            ),
                        )
                        .child(pixel_text(
                            format!("{}%", (preview.zoom * 100.0) as u32),
                            p.muted,
                        ))
                        .child(
                            button("attachment-preview-zoom-in", "+", p, |_| {}).on_press_tx(
                                |s: &mut State, tx| {
                                    if let Some(preview) = &mut s.attachment_preview {
                                        preview.change_zoom(preview.zoom * 1.5, None, tx);
                                    }
                                },
                            ),
                        ),
                );
            }
            Kind::Video | Kind::Audio => {
                if let Some(player) = &preview.player {
                    if attachment_kind == Kind::Video {
                        card = card.child(
                            el().key("attachment-preview-video")
                                .w_full()
                                .h(0.0)
                                .grow(1.0)
                                .min_h(0.0)
                                .video(player, rune::SurfaceFit::Fit),
                        );
                    } else {
                        card = card.child(icon("file", p.muted).key("attachment-preview-audio"));
                    }
                    let status = player.state();
                    if let rune::PlayerState::Failed(_) = status {
                        card = card
                            .child(
                                label(
                                    format!("Video unavailable · {}", attachment.name),
                                    BODY,
                                    p.muted,
                                )
                                .wrap(),
                            )
                            .child(button("retry-video", "Retry video", p, |s| {
                                if let Some(preview) = &mut s.attachment_preview {
                                    preview.load(s.wake.clone());
                                }
                            }));
                    }
                    let player_copy = player.clone();
                    let player_mute = player.clone();
                    let player_seek = player.clone();
                    let player_step = player.clone();
                    let duration = player.duration().unwrap_or(0.0);
                    let fraction = if duration > 0.0 {
                        (player.time() / duration).clamp(0.0, 1.0) as f32
                    } else {
                        0.0
                    };
                    card = card.child(
                        el().key("attachment-preview-seek")
                            .role("slider")
                            .label("Playback position")
                            .a11y_value(format!("{:.0} seconds", player.time()))
                            .focusable(true)
                            .w_full()
                            .min_w(0.0)
                            .h(20.0)
                            .cursor(rune::Cursor::Pointer)
                            .on_step(move |_: &mut State, delta, _| {
                                player_step.seek(player_step.time() + f64::from(delta) * 5.0)
                            })
                            .on_drag(move |_: &mut State, drag, _| {
                                if !drag.cancelled() {
                                    player_seek.seek(
                                        (f64::from(drag.x / drag.w.max(1.0))
                                            * player_seek.duration().unwrap_or(0.0))
                                        .max(0.0),
                                    );
                                }
                            })
                            .child(
                                row()
                                    .w_full()
                                    .h(6.0)
                                    .fill(p.selected)
                                    .child(el().w(0.0).grow(fraction).h_full().fill(p.sea))
                                    .child(el().w(0.0).grow(1.0 - fraction).h_full()),
                            ),
                    );
                    let player_volume = player.clone();
                    let player_rate = player.clone();
                    card = card.child(
                        row()
                            .key("attachment-preview-media-controls")
                            .center_y()
                            .gap(8.0)
                            .child(button(
                                "attachment-preview-play",
                                if player.is_playing() { "Pause" } else { "Play" },
                                p,
                                move |_| {
                                    if player_copy.is_playing() {
                                        player_copy.pause();
                                    } else {
                                        player_copy.play();
                                    }
                                },
                            ))
                            .child(button(
                                "attachment-preview-mute",
                                if player.muted() { "Unmute" } else { "Mute" },
                                p,
                                move |_| player_mute.set_muted(!player_mute.muted()),
                            ))
                            .child(pixel_text(
                                format!(
                                    "{:.0} / {:.0} s",
                                    player.time(),
                                    player.duration().unwrap_or(0.0)
                                ),
                                p.muted,
                            ))
                            .child(button(
                                "attachment-preview-volume",
                                "Volume",
                                p,
                                move |_| {
                                    player_volume.set_volume(if player_volume.volume() >= 1.0 {
                                        0.5
                                    } else {
                                        1.0
                                    })
                                },
                            ))
                            .child(button(
                                "attachment-preview-speed",
                                &format!("{}×", player.rate()),
                                p,
                                move |_| {
                                    player_rate.set_rate(if player_rate.rate() >= 2.0 {
                                        0.5
                                    } else {
                                        player_rate.rate() + 0.5
                                    })
                                },
                            ))
                            .child(
                                icon_button(
                                    "attachment-preview-fullscreen",
                                    "Full screen",
                                    "expand",
                                    p,
                                    |_| {},
                                )
                                .on_press_tx(|_: &mut State, tx| tx.toggle_full_screen()),
                            ),
                    );
                }
            }
            Kind::Text | Kind::Markdown | Kind::Html => {
                let text = preview.text.clone();
                let document = preview.document.clone();
                let mut actions = row()
                    .gap(8.0)
                    .center_y()
                    .child(
                        el().key("copy-attachment-preview")
                            .role("button")
                            .label("Copy contents")
                            .focusable(true)
                            .cursor(rune::Cursor::Pointer)
                            .on_press_tx(move |s: &mut State, tx| {
                                if let Some(document) = &document {
                                    if !document.complete {
                                        s.push_toast(crate::model::ToastKind::Error, "Could not copy file",
                                            Some("Wait for the file to finish loading before copying.".into()), None, Some(6000));
                                    } else if let Ok(text) = document.text() { tx.copy(text); }
                                } else { tx.copy(&text); }
                            })
                            .child(label("Copy contents", LABEL, p.ink)),
                    )
                    .child(button("wrap-attachment-preview", "Word wrap", p, |s| {
                        if let Some(preview) = &mut s.attachment_preview {
                            preview.wrap = !preview.wrap;
                        }
                    }));
                if attachment_kind == Kind::Markdown {
                    actions = actions.child(button(
                        "render-attachment-preview",
                        if preview.rendered {
                            "Show markdown source"
                        } else {
                            "Show rendered markdown"
                        },
                        p,
                        |s| {
                            if let Some(preview) = &mut s.attachment_preview {
                                preview.rendered = !preview.rendered;
                            }
                        },
                    ));
                }
                if attachment_kind == Kind::Html {
                    let file = attachment.clone();
                    actions = actions.child(button(
                        "render-attachment-preview",
                        "Show rendered page",
                        p,
                        move |s| {
                            s.html_windows.open_attachment_async(
                                file.clone(),
                                s.dark,
                                s.wake.clone(),
                            )
                        },
                    ));
                }
                if attachment_text_preview::delimiter(&attachment.name, &attachment.mime).is_some()
                {
                    actions = actions.child(button(
                        "render-attachment-preview",
                        if preview.rendered {
                            "Show source"
                        } else {
                            "Show table"
                        },
                        p,
                        |s| s.toggle_attachment_table(),
                    ));
                }
                card = card.child(actions);
                if let Some(document) = &preview.document {
                    let table =
                        attachment_text_preview::delimiter(&attachment.name, &attachment.mime)
                            .is_some()
                            && preview.rendered;
                    let count = document.len();
                    let columns = if table { document.columns } else { 1 };
                    let column_width =
                        ((width - 56.0) / columns.min(6).max(1) as f32).clamp(128.0, 320.0);
                    card = card.child(pixel_text(
                        format!(
                            "{} {}{}{}",
                            count.saturating_sub(usize::from(table && count > 0)),
                            if table { "rows" } else { "lines" },
                            if table {
                                format!(" · {columns} columns")
                            } else {
                                String::new()
                            },
                            if document.complete {
                                String::new()
                            } else {
                                " · Indexing…".into()
                            }
                        ),
                        p.muted,
                    ));
                    let list = col()
                        .key("attachment-text-rows")
                        .growth(Default::default())
                        .virtual_rows(
                            count,
                            29.0,
                            |_: &State, i| format!("attachment-text-row-{i}").into(),
                            move |s: &State, i| {
                                let Some(preview) = &s.attachment_preview else {
                                    return el();
                                };
                                let Some(document) = &preview.document else {
                                    return el();
                                };
                                let cells = match document.row(i, table) {
                                    Ok(cells) => cells,
                                    Err(error) => {
                                        return label(error, SMALL, palette(s.dark).danger).wrap();
                                    }
                                };
                                let mut line = row().min_h(29.0).min_w(0.0).child(
                                    label(
                                        format!("{}", i + usize::from(!table)),
                                        SMALL,
                                        palette(s.dark).muted,
                                    )
                                    .w(56.0)
                                    .px(8.0)
                                    .py(6.0),
                                );
                                for cell in cells {
                                    let label = label(
                                        cell,
                                        TextStyle::new(12.0, 400.0)
                                            .family(rune::Family::SystemMono),
                                        palette(s.dark).ink,
                                    )
                                    .px(12.0)
                                    .py(6.0)
                                    .selectable(true);
                                    line = line.child(if table {
                                        label.w(column_width).wrap_anywhere()
                                    } else if preview.wrap {
                                        label.w(0.0).grow(1.0).min_w(0.0).wrap_anywhere()
                                    } else {
                                        label
                                    });
                                }
                                line
                            },
                        );
                    card = card.child(
                        col()
                            .key("attachment-preview-file")
                            .w_full()
                            .min_w(0.0)
                            .h(0.0)
                            .grow(1.0)
                            .min_h(0.0)
                            .scroll_xy()
                            .child(list),
                    );
                    if !document.complete {
                        card = card.child(label("Loading the rest of the file…", SMALL, p.muted));
                    }
                    if document.columns == 1024 {
                        card = card.child(label(
                        "Showing the first 1,024 columns. Source and Save retain every column.", SMALL, p.muted).wrap());
                    }
                    // The virtual list owns its rows. Do not also render a full text node.
                } else {
                    let body = if attachment_kind == Kind::Markdown && preview.rendered {
                        markdown_ui::render_with_find(
                            &preview.text,
                            p,
                            s.dark,
                            width > 760.0,
                            false,
                            false,
                            "attachment-preview-markdown",
                            None,
                        )
                    } else {
                        let text = label(
                            &preview.text,
                            TextStyle::new(13.0, 400.0).family(rune::Family::SystemMono),
                            p.ink,
                        )
                        .key("attachment-preview-text")
                        .selectable(true);
                        if preview.wrap {
                            text.wrap_anywhere().w_full()
                        } else {
                            text
                        }
                    };
                    card = card.child(
                        col()
                            .key("attachment-preview-file")
                            .w_full()
                            .min_w(0.0)
                            .h(0.0)
                            .grow(1.0)
                            .min_h(0.0)
                            .scroll_xy()
                            .child(body),
                    );
                    if preview.truncated {
                        card =
                            card.child(label("Preview limited to the first 1 MB.", SMALL, p.muted));
                    }
                }
            }
            Kind::Sqlite => {
                if let Some(database) = &preview.database {
                    if let Some(table) = database.tables.get(preview.database_table) {
                        let page = preview.database_page_index;
                        let table_index = preview.database_table;
                        let busy = preview.database_loading;
                        card = card.child(
                            row()
                                .key("attachment-database-controls")
                                .w_full()
                                .gap(8.0)
                                .center_y()
                                .child(
                                    button(
                                        "attachment-database-table",
                                        &format!("{} ({} rows)", table.name, table.count),
                                        p,
                                        |s| {
                                            if let Some(preview) = &mut s.attachment_preview {
                                                preview.database_picker = !preview.database_picker;
                                            }
                                        },
                                    )
                                    .disabled(busy)
                                    .w(0.0)
                                    .grow(1.0)
                                    .min_w(128.0),
                                )
                                .child(
                                    button(
                                        "attachment-database-previous",
                                        "Previous rows",
                                        p,
                                        move |s| {
                                            s.load_attachment_database(
                                                table_index,
                                                page.saturating_sub(1),
                                            )
                                        },
                                    )
                                    .disabled(busy || page == 0),
                                )
                                .child(
                                    button("attachment-database-next", "Next rows", p, move |s| {
                                        s.load_attachment_database(table_index, page + 1)
                                    })
                                    .disabled(busy || (page + 1) as u64 * 50 >= table.count),
                                ),
                        );
                        card = card.child(label(
                            if table.count == 0 {
                                "0 rows".into()
                            } else {
                                format!(
                                    "{}–{} of {}",
                                    page * 50 + 1,
                                    ((page + 1) as u64 * 50).min(table.count),
                                    table.count
                                )
                            },
                            SMALL,
                            p.muted,
                        ));
                        if preview.database_picker {
                            let mut picker = col()
                                .key("attachment-database-tables")
                                .max_h(200.0)
                                .scroll(ScrollSpec {
                                    stick_bottom: false,
                                });
                            for (index, table) in database.tables.iter().enumerate() {
                                picker = picker.child(button(
                                    &format!("attachment-database-table-{index}"),
                                    &table.name,
                                    p,
                                    move |s| s.load_attachment_database(index, 0),
                                ));
                            }
                            card = card.child(picker);
                        }
                    }
                    card = card.child(label("Read-only. Up to 50 columns and 1,024 characters per cell. Blob contents are omitted.", SMALL, p.muted).wrap());
                    if let Some(page) = &preview.database_page {
                        let mut table =
                            col().key("attachment-preview-database").min_w(0.0).gap(0.0);
                        let mut header = row().min_h(29.0);
                        for name in &page.columns {
                            header = header.child(
                                label(name, SMALL, p.ink)
                                    .w(160.0)
                                    .px(12.0)
                                    .py(6.0)
                                    .wrap_anywhere(),
                            );
                        }
                        table = table.child(header);
                        for cells in &page.rows {
                            let mut line = row().min_h(29.0);
                            for cell in cells {
                                line = line.child(
                                    label(cell, SMALL, p.ink)
                                        .w(160.0)
                                        .px(12.0)
                                        .py(6.0)
                                        .wrap_anywhere()
                                        .selectable(true),
                                );
                            }
                            table = table.child(line);
                        }
                        card = card.child(
                            col()
                                .h(0.0)
                                .grow(1.0)
                                .min_h(0.0)
                                .w_full()
                                .min_w(0.0)
                                .scroll_xy()
                                .child(table),
                        );
                    }
                } else {
                    card = card.child(label("Loading…", SMALL, p.muted));
                }
            }
            Kind::Archive => {
                card = card.child(archive_view(s, p));
            }
            Kind::Notebook => {
                card = card.child(notebook_view(s, p, width));
            }
            Kind::Pdf | Kind::Font => {
                let file = attachment.clone();
                card = card.child(button(
                    "open-native-attachment-preview",
                    "Preview",
                    p,
                    move |s| {
                        s.html_windows
                            .open_attachment_async(file.clone(), s.dark, s.wake.clone())
                    },
                ));
            }
            _ => {
                card = card
                    .child(
                        label("No preview for this file", BODY, p.muted)
                            .key("attachment-preview-unsupported"),
                    )
                    .child(
                        label(
                            format!(
                                "Save it to open in an app that supports {} files.",
                                attachment.name.rsplit('.').next().unwrap_or("this format")
                            ),
                            SMALL,
                            p.muted,
                        )
                        .wrap(),
                    );
            }
        }
    }
    if preview.items.len() > 1 {
        card = card.child(
            row()
                .gap(8.0)
                .justify_center()
                .child(button(
                    "previous-attachment-preview",
                    "Previous media",
                    p,
                    |s| s.navigate_attachment_preview(-1),
                ))
                .child(pixel_text(
                    format!("{}/{}", preview.index + 1, preview.items.len()),
                    p.muted,
                ))
                .child(button("next-attachment-preview", "Next media", p, |s| {
                    s.navigate_attachment_preview(1)
                })),
        );
    }
    el().key("attachment-preview-overlay")
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .fill(Color::BLACK.alpha(0.35))
        .center()
        .on_press(|s: &mut State| s.attachment_preview = None)
        .modal(true, |s: &mut State| s.attachment_preview = None)
        .on_key(|s: &mut State, event, tx| {
            if !event.mods.ctrl
                && !event.mods.cmd
                && !event.mods.alt
                && let Some(preview) = &mut s.attachment_preview
                && kind(&preview.selected().name, &preview.selected().mime) == Kind::Image
            {
                let next = match &event.key {
                    rune::Key::Enter => Some(if preview.zoom > 1.0 { 1.0 } else { 2.0 }),
                    rune::Key::Text(text) => match text.as_str() {
                        " " => Some(if preview.zoom > 1.0 { 1.0 } else { 2.0 }),
                        "+" | "=" => Some(preview.zoom * 1.5),
                        "-" => Some(preview.zoom / 1.5),
                        "0" => Some(1.0),
                        _ => None,
                    },
                    _ => None,
                };
                if let Some(next) = next {
                    preview.change_zoom(next, None, tx);
                    return true;
                }
                let scroll = preview.scroll();
                let delta = match event.key {
                    rune::Key::Left if scroll.max[0] > 0.0 => Some([-40.0, 0.0]),
                    rune::Key::Right if scroll.max[0] > 0.0 => Some([40.0, 0.0]),
                    rune::Key::Up => Some([0.0, -40.0]),
                    rune::Key::Down => Some([0.0, 40.0]),
                    _ => None,
                };
                if preview.zoom > 1.0
                    && let Some(delta) = delta
                {
                    tx.scroll_to_instant(
                        "attachment-preview-image-viewport",
                        scroll.offset[0] + delta[0],
                        scroll.offset[1] + delta[1],
                    );
                    return true;
                }
            }
            match event.key {
                rune::Key::Left => s.navigate_attachment_preview(-1),
                rune::Key::Right => s.navigate_attachment_preview(1),
                _ => return false,
            }
            true
        })
        .child(el().key("transcript-context-preview").child(card))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_keeps_the_pointer_anchor_and_clamps_like_ksg() {
        assert_eq!(
            zoom_offset(1.0, 2.0, [20.0, 30.0], [40.0, 50.0]),
            [80.0, 110.0]
        );
        assert_eq!(
            zoom_offset(2.0, 1.0, [80.0, 110.0], [40.0, 50.0]),
            [20.0, 30.0]
        );
        assert_eq!(clamped_zoom(20.0), 8.0);
        assert_eq!(clamped_zoom(0.1), 1.0);
    }

    #[test]
    fn preview_kinds_keep_video_files_and_images_distinct() {
        assert_eq!(kind("clip.mov", "application/octet-stream"), Kind::Video);
        assert_eq!(kind("clip.mp4", "video/mp4"), Kind::Video);
        assert_eq!(kind("drawing.svg", "image/svg+xml"), Kind::Image);
        assert_eq!(kind("notes.md", "text/plain"), Kind::Markdown);
        assert_eq!(kind("report.pdf", "application/pdf"), Kind::Pdf);
        assert_eq!(kind("sound.wav", "audio/wav"), Kind::Audio);
        for name in [
            "Dockerfile",
            ".env.local",
            "main.cpp",
            "sample.ndjson",
            "Gemfile",
            "app.vue",
        ] {
            assert_eq!(kind(name, "application/octet-stream"), Kind::Text, "{name}");
        }
        assert_eq!(kind("data.json", "application/problem+json"), Kind::Text);
        assert_eq!(kind("voice.opus", ""), Kind::Audio);
        assert_eq!(kind("not-a-database.db", "image/png"), Kind::Image);
        assert_eq!(
            kind("file.bin", "application/octet-stream"),
            Kind::Unsupported
        );
    }
}
