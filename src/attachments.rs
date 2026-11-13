//! Composer attachments. Images and transferred files use owned blobs.
//! Local ordinary files and path mentions retain absolute path references until transfer.
//!
//! Pi accepts images as typed prompt parts: `{ type: "image", data, mimeType }`. Shina keeps
//! the bytes in an immutable, Shina-owned blob store under its local data directory and keeps
//! only a small reference in drafts and state. Base64 exists only while a prompt is encoded for
//! Pi, so saved Shina JSON never grows with image payloads.
use image::{ImageFormat, ImageReader};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

pub const MAX_ATTACHMENTS: usize = 100;
/// KSG contracts/chatAttachment.PROVIDER_SEND_TURN_MAX_FILE_BYTES.
pub const MAX_FILE_BYTES: u64 = 50 * 1024 * 1024;
pub const MAX_TEXT_BYTES: u64 = 256 * 1024;
/// The largest single image Shina accepts, before base64.
pub const MAX_IMAGE_BYTES: u64 = 10 * 1024 * 1024;
/// The largest combined image payload in one draft.
pub const MAX_TOTAL_BYTES: u64 = 80 * 1024 * 1024;
pub fn legacy_peer_limit_error(images: usize, bytes: u64) -> Option<String> {
    if images > 8 {
        Some(
            "This server supports up to 8 images per message. Update it to send more images."
                .into(),
        )
    } else if bytes > 20 * 1024 * 1024 {
        Some("This server supports up to 20 MiB of images per message. Update it to send larger image batches.".into())
    } else {
        None
    }
}
const NAME_LIMIT: usize = 255;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Attachment {
    /// A content-addressed blob filename, or a hash of a local path reference.
    pub id: String,
    /// The name shown in a chip. Display only, never used as a path.
    pub name: String,
    pub mime: String,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
    /// An ordinary file or folder, including a staged file's owned blob path.
    pub path: Option<PathBuf>,
    /// A draft keeps its logical identity while preparation and upload replace its bytes.
    /// Removed at the transport boundary for older peers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uploaded: Option<Uploaded>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<AttachmentSource>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "_tag")]
pub enum AttachmentSource {
    #[serde(rename = "pasted-text")]
    PastedText,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Uploaded {
    pub environment_id: String,
    pub lease_id: String,
    pub attachment_id: String,
}

impl Attachment {
    /// KSG composerDraftStore.composerFileNeedsReattach.
    pub fn needs_reattach(&self) -> bool {
        !self.is_image()
            && self.mime != "inode/directory"
            && self.uploaded.is_none()
            && !path_for(self).is_some_and(|path| {
                fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() == self.bytes)
            })
    }

    /// KSG composerDraftStore.composerFileMatchesReattachMarker.
    pub fn matches_reattach(&self, name: &str, mime: &str, bytes: u64) -> bool {
        if !self.needs_reattach() || self.name != name || self.bytes != bytes {
            return false;
        }
        self.mime == mime
            || crate::attachment_intake::file_mime(&self.name, &self.mime) == "image/x-adobe-dng"
                && crate::attachment_intake::file_mime(name, mime) == "image/x-adobe-dng"
            || matches!(
                self.mime.to_ascii_lowercase().as_str(),
                "" | "application/octet-stream"
            ) && crate::attachment_intake::file_mime(&self.name, &self.mime)
                .starts_with("video/")
                && crate::attachment_intake::file_mime(name, mime).starts_with("video/")
    }

    pub fn asset_id(&self) -> &str {
        self.asset_id.as_deref().unwrap_or(&self.id)
    }
    pub fn is_image(&self) -> bool {
        self.path.is_none()
    }

    pub fn to_value(&self) -> Value {
        json!({
            "id": self.id,
            "name": self.name,
            "mime": self.mime,
            "width": self.width,
            "height": self.height,
            "bytes": self.bytes,
            "path": self.path,
            "asset_id": self.asset_id,
            "uploaded": self.uploaded,
            "source": self.source,
        })
    }

    /// Restore image blobs only at their expected size. Retain missing ordinary-file references.
    pub fn from_value(value: &Value) -> Option<Self> {
        // Local draft-only fields never change an older peer's attachment schema.
        if let Some(asset_id) = value["asset_id"].as_str() {
            let id = value["id"].as_str()?;
            if !(safe_id(id) && safe_id(asset_id) || safe_file_id(id) && safe_file_id(asset_id)) {
                return None;
            }
            let mut source = value.clone();
            source.as_object_mut()?.remove("asset_id");
            source["id"] = json!(asset_id);
            let mut attachment = Self::from_value(&source)?;
            attachment.id = id.into();
            attachment.asset_id = Some(asset_id.into());
            return Some(attachment);
        }
        let uploaded = value
            .get("uploaded")
            .filter(|value| !value.is_null())
            .and_then(|value| serde_json::from_value::<Uploaded>(value.clone()).ok())
            .filter(|upload| {
                !upload.environment_id.is_empty()
                    && uuid::Uuid::parse_str(&upload.lease_id).is_ok()
                    && (safe_id(&upload.attachment_id) || safe_file_id(&upload.attachment_id))
            });
        let id = value["id"].as_str()?;
        let source = serde_json::from_value::<AttachmentSource>(value["source"].clone()).ok();
        if safe_file_id(id) {
            let name = value["name"].as_str()?;
            let mime = value["mime"].as_str()?;
            let bytes = value["bytes"].as_u64()?;
            validate_file_metadata(name, mime, bytes).ok()?;
            let path = value["path"]
                .as_str()
                .map(PathBuf::from)
                .filter(|path| {
                    path.is_absolute()
                        && path.file_name().is_some_and(|file| file == id)
                        && fs::symlink_metadata(path).is_ok_and(|m| m.is_file() && m.len() == bytes)
                })
                .or_else(|| directory().map(|directory| directory.join(id)))?;
            return Some(Self {
                id: id.into(),
                name: name.into(),
                mime: mime.into(),
                width: 0,
                height: 0,
                bytes,
                path: Some(path),
                asset_id: None,
                uploaded,
                source,
            });
        }
        if let Some(path) = value["path"].as_str() {
            let path = PathBuf::from(path);
            let name = value["name"].as_str()?;
            let mime = value["mime"].as_str()?;
            if !path.is_absolute()
                || name.is_empty()
                || name.chars().count() > NAME_LIMIT
                || mime.trim().is_empty()
                || mime.encode_utf16().count() > 100
                || id != file_id(&path)
            {
                return None;
            }
            // Keep missing references visible and removable after restoring a draft.
            return Some(Self {
                id: id.into(),
                name: name.into(),
                mime: mime.into(),
                width: 0,
                height: 0,
                bytes: value["bytes"].as_u64()?,
                path: Some(path),
                asset_id: None,
                uploaded,
                source,
            });
        }
        if !safe_id(id) {
            return None;
        }
        let name = value["name"].as_str()?;
        let mime = value["mime"].as_str()?;
        if !matches!(
            mime,
            "image/png" | "image/jpeg" | "image/gif" | "image/webp"
        ) || name.is_empty()
            || name.chars().count() > NAME_LIMIT
        {
            return None;
        }
        let width = u32::try_from(value["width"].as_u64()?).ok()?;
        let height = u32::try_from(value["height"].as_u64()?).ok()?;
        let bytes = value["bytes"].as_u64()?;
        if width == 0 || height == 0 || bytes == 0 || bytes > MAX_IMAGE_BYTES {
            return None;
        }
        let path = path_for_id(id)?;
        let metadata = fs::metadata(path).ok()?;
        (metadata.is_file() && metadata.len() == bytes).then_some(Self {
            id: id.into(),
            name: name.into(),
            mime: mime.into(),
            width,
            height,
            bytes,
            path: None,
            asset_id: None,
            uploaded,
            source,
        })
    }
}

/// A prompt snapshot. Editing the draft after a send never changes these references.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prompt {
    pub text: String,
    pub images: Arc<[Attachment]>,
    pub context: Option<crate::composer_context::MessageContext>,
    pub draft: Option<crate::composer_rich::Snapshot>,
}

impl Prompt {
    pub fn new(text: String, images: Vec<Attachment>) -> Self {
        Self {
            text,
            images: images.into(),
            context: None,
            draft: None,
        }
    }

    pub fn with_context(
        text: String,
        images: Vec<Attachment>,
        context: Option<crate::composer_context::MessageContext>,
    ) -> Self {
        Self {
            text,
            images: images.into(),
            context,
            draft: None,
        }
    }
}

pub fn directory() -> Option<PathBuf> {
    crate::store::directory().map(|directory| directory.join("attachments"))
}

pub fn path_for(attachment: &Attachment) -> Option<PathBuf> {
    attachment
        .path
        .clone()
        .or_else(|| path_for_id(attachment.asset_id()))
}

pub fn preview_path_for(attachment: &Attachment) -> Option<PathBuf> {
    if !attachment.is_image() {
        return None;
    }
    let path = path_for_id(&format!("preview-{}.png", attachment.asset_id()))?;
    path.is_file().then_some(path)
}

fn path_for_id(id: &str) -> Option<PathBuf> {
    (safe_id(id) || safe_file_id(id)).then(|| directory().map(|directory| directory.join(id)))?
}

/// Ordinary files have immutable, content-addressed identities, distinct from image IDs
/// and legacy path references. No caller-supplied filename is a storage path.
pub(crate) fn safe_file_id(id: &str) -> bool {
    id.strip_prefix("file-")
        .and_then(|id| id.strip_suffix(".bin"))
        .is_some_and(|hash| {
            hash.len() == 64
                && hash
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
}

fn validate_file_metadata(name: &str, mime: &str, length: u64) -> Result<(), String> {
    if name.trim().is_empty()
        || name.encode_utf16().count() > 255
        || mime.trim().is_empty()
        || mime.encode_utf16().count() > 100
    {
        return Err("Invalid file attachment metadata.".into());
    }
    if length == 0 {
        return Err(format!("'{name}' is empty or could not be read."));
    }
    if length > MAX_FILE_BYTES {
        return Err(format!("'{name}' exceeds the 50 MB attachment limit."));
    }
    Ok(())
}

/// Ported from KSG assets/AttachmentUpload.storeAttachmentUpload. Publish without
/// replacement; a duplicate must contain the same bytes. Shina uses SHA-256 IDs.
pub(crate) fn store_uploaded_file(
    directory: &Path,
    name: &str,
    mime: &str,
    bytes: &[u8],
) -> Result<Attachment, String> {
    use sha2::Digest;
    use std::io::Write;
    validate_file_metadata(name, mime, bytes.len() as u64)?;
    fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    let directory = directory.canonicalize().map_err(|e| e.to_string())?;
    let id = format!("file-{:x}.bin", sha2::Sha256::digest(bytes));
    let path = directory.join(&id);
    let staged = directory.join(format!("{}.part", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = fs::File::create_new(&staged).map_err(|e| e.to_string())?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        match fs::hard_link(&staged, &path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if fs::symlink_metadata(&path)
                    .is_ok_and(|m| m.is_file() && m.len() == bytes.len() as u64)
                    && fs::read(&path).is_ok_and(|existing| existing == bytes)
                {
                    Ok(())
                } else {
                    Err("Attachment already uploaded.".into())
                }
            }
            Err(e) => Err(e.to_string()),
        }
    })();
    let _ = fs::remove_file(&staged);
    result?;
    Ok(Attachment {
        id,
        name: name.trim().into(),
        mime: mime.trim().into(),
        width: 0,
        height: 0,
        bytes: bytes.len() as u64,
        path: Some(path),
        asset_id: None,
        uploaded: None,
        source: None,
    })
}

pub(crate) fn transfer_file_bytes(attachment: &Attachment) -> Result<Vec<u8>, String> {
    validate_file_metadata(&attachment.name, &attachment.mime, attachment.bytes)?;
    let path = path_for(attachment).ok_or("The original attachment is no longer available.")?;
    let file =
        fs::File::open(&path).map_err(|_| "The original attachment is no longer available.")?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Folders cannot be transferred as file attachments.".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    validate_file_metadata(&attachment.name, &attachment.mime, bytes.len() as u64)?;
    if safe_file_id(attachment.asset_id()) {
        use sha2::Digest;
        if attachment.asset_id() != format!("file-{:x}.bin", sha2::Sha256::digest(&bytes)) {
            return Err("File hash mismatch.".into());
        }
    }
    Ok(bytes)
}

pub(crate) fn stage_transfer_file(attachment: &Attachment) -> Result<Attachment, String> {
    let bytes = transfer_file_bytes(attachment)?;
    let mut staged = store_uploaded_file(
        &directory().ok_or("The attachment directory is unavailable.")?,
        &attachment.name,
        &attachment.mime,
        &bytes,
    )?;
    staged.source = attachment.source.clone();
    Ok(staged)
}

/// Stage only submitted/transferred files. Local path mentions and draft editing keep
/// their existing path-reference behavior; sent records never depend on that source path.
pub(crate) fn stage_prompt_files(prompt: &mut Prompt) -> Result<(), String> {
    validate_prompt_images(&prompt.images)?;
    let mut replacements = std::collections::HashMap::new();
    let attached_ids = prompt.draft.as_ref().map(|draft| {
        draft
            .attachments
            .iter()
            .map(|a| a.id.clone())
            .chain(
                prompt
                    .context
                    .iter()
                    .flat_map(|context| context.records.iter())
                    .filter_map(|r| match &r.data {
                        crate::composer_context::ComposerContextData::File {
                            attachment_id,
                            ..
                        } => Some(attachment_id.clone()),
                        _ => None,
                    }),
            )
            .collect::<std::collections::HashSet<_>>()
    });
    for attachment in Arc::make_mut(&mut prompt.images) {
        if attachment.is_image() && attachment.asset_id.is_some() {
            let mut normalized = attachment.clone();
            normalized.id = normalized.asset_id.take().unwrap();
            normalized.uploaded = None;
            replacements.insert(attachment.id.clone(), normalized.clone());
            *attachment = normalized;
        }
        if attached_ids
            .as_ref()
            .is_some_and(|ids| !ids.contains(&attachment.id))
        {
            continue; // Legacy path mentions are not uploaded file attachments.
        }
        let owned = directory().and_then(|d| d.join(&attachment.id).canonicalize().ok());
        let already_staged = safe_file_id(&attachment.id)
            && owned.is_some()
            && attachment.path.as_ref().and_then(|p| p.canonicalize().ok()) == owned;
        if !attachment.is_image() && attachment.mime != "inode/directory" && !already_staged {
            let staged = stage_transfer_file(attachment)?;
            replacements.insert(attachment.id.clone(), staged.clone());
            *attachment = staged;
        }
    }
    fn rewrite(
        records: &mut [crate::composer_context::ComposerContextRecord],
        replacements: &std::collections::HashMap<String, Attachment>,
    ) {
        for record in records {
            if let crate::composer_context::ComposerContextData::File {
                attachment_id,
                name,
                mime_type,
                size_bytes,
            }
            | crate::composer_context::ComposerContextData::Image {
                attachment_id,
                name,
                mime_type,
                size_bytes,
            } = &mut record.data
                && let Some(attachment) = replacements.get(attachment_id)
            {
                *attachment_id = attachment.id.clone();
                *name = attachment.name.clone();
                *mime_type = attachment.mime.clone();
                *size_bytes = attachment.bytes;
            }
        }
    }
    if let Some(context) = &mut prompt.context {
        rewrite(&mut context.records, &replacements);
    }
    if let Some(draft) = &mut prompt.draft {
        for attachment in &mut draft.attachments {
            if let Some(staged) = replacements.get(&attachment.id) {
                *attachment = staged.clone();
            }
        }
        rewrite(&mut draft.context, &replacements);
    }
    Ok(())
}

pub(crate) fn stage_clipboard_files(
    records: &mut Vec<crate::composer_context::ComposerContextRecord>,
    files: &mut std::collections::HashMap<String, Attachment>,
) -> Result<(), String> {
    let mut prompt = Prompt::with_context(
        String::new(),
        files.values().cloned().collect(),
        Some(crate::composer_context::MessageContext {
            version: 1,
            records: records.clone(),
        }),
    );
    stage_prompt_files(&mut prompt)?;
    *records = prompt.context.unwrap().records;
    *files = prompt
        .images
        .iter()
        .map(|file| (file.id.clone(), file.clone()))
        .collect();
    Ok(())
}

/// A blob id is one filename with a supported image extension.
pub(crate) fn safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 96
        && !id.starts_with('.')
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        && id
            .rsplit_once('.')
            .is_some_and(|(_, extension)| matches!(extension, "png" | "jpg" | "gif" | "webp"))
}

fn file_id(path: &Path) -> String {
    use sha2::Digest;
    format!(
        "file-{:x}",
        sha2::Sha256::digest(path.to_string_lossy().as_bytes())
    )
}

/// Detect text by content, never by filename. Bound reads even when the file grows.
fn text_content(path: &Path) -> Result<Option<String>, String> {
    let file =
        fs::File::open(path).map_err(|e| format!("Could not read {}: {e}", path.display()))?;
    if file.metadata().map_err(|e| e.to_string())?.len() > MAX_TEXT_BYTES {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    file.take(MAX_TEXT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    Ok(
        (bytes.len() as u64 <= MAX_TEXT_BYTES && !bytes.contains(&0))
            .then(|| String::from_utf8(bytes).ok())
            .flatten(),
    )
}

/// Validate images, or retain an absolute reference to an ordinary file or folder.
pub fn attach_file(path: &Path) -> Result<Attachment, String> {
    let path =
        fs::canonicalize(path).map_err(|e| format!("Missing file {}: {e}", path.display()))?;
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Image".into());
    let read_error = |error| {
        format!(
            "Could not read {}. Check that it still exists and is a file: {error}",
            clean_name(&name)
        )
    };
    let metadata = fs::metadata(&path).map_err(read_error)?;
    if !metadata.is_file() && !metadata.is_dir() {
        return Err("Choose a file or folder, not a device.".into());
    }
    if metadata.is_file() {
        let mime = crate::attachment_intake::native_mime(&name);
        match crate::attachment_intake::classify(&name, mime) {
            crate::attachment_intake::Kind::Image => return attach_image_file(&path),
            crate::attachment_intake::Kind::UnsupportedImage => {
                return Err(format!(
                    "'{name}' is not a supported image type. Attach GIF, HEIC, HEIF, JPEG, PNG, or WebP images."
                ));
            }
            crate::attachment_intake::Kind::File => {}
        }
        validate_file_metadata(&name, mime, metadata.len())?;
        let mut attachment = file_reference(path, &metadata, &name)?;
        attachment.mime = crate::attachment_intake::file_mime(&name, mime);
        return Ok(attachment);
    }
    file_reference(path, &metadata, &name)
}

/// An @ mention always refers to a path, including image paths. It does not request image input.
pub fn reference_file(path: &Path) -> Result<Attachment, String> {
    let path =
        fs::canonicalize(path).map_err(|e| format!("Missing file {}: {e}", path.display()))?;
    let metadata = fs::metadata(&path).map_err(|e| e.to_string())?;
    if !metadata.is_file() && !metadata.is_dir() {
        return Err("Choose a file or folder, not a device.".into());
    }
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    file_reference(path, &metadata, &name)
}

fn file_reference(
    path: PathBuf,
    metadata: &fs::Metadata,
    name: &str,
) -> Result<Attachment, String> {
    let mime = if metadata.is_dir() {
        "inode/directory"
    } else if text_content(&path)?.is_some() {
        "text/plain"
    } else {
        "application/octet-stream"
    };
    Ok(Attachment {
        id: file_id(&path),
        name: clean_name(name),
        mime: mime.into(),
        width: 0,
        height: 0,
        bytes: metadata.len(),
        path: Some(path),
        asset_id: None,
        uploaded: None,
        source: None,
    })
}

fn attach_image_file(path: &Path) -> Result<Attachment, String> {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let metadata = fs::metadata(path).map_err(|e| e.to_string())?;
    if metadata.len() > crate::attachment_intake::MAX_SOURCE_BYTES as u64 {
        return Err(crate::attachment_intake::Failure::TooLarge.message(&name));
    }
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    // Bound the read as well as checking metadata, since the file can grow while we read it.
    let mut bytes = Vec::new();
    file.take(crate::attachment_intake::MAX_SOURCE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Could not read {}: {error}", clean_name(&name)))?;
    let (name, bytes) = crate::attachment_intake::prepare(
        &name,
        crate::attachment_intake::native_mime(&name),
        bytes,
        MAX_IMAGE_BYTES as usize,
    )
    .map_err(|failure| failure.message(&name))?;
    attach_bytes(&name, bytes)
}

/// Validate and store image bytes, with the name shown in the composer chip.
pub fn attach_bytes(name: &str, bytes: Vec<u8>) -> Result<Attachment, String> {
    let name = clean_name(name);
    use sha2::Digest;
    let cache = directory().map(|dir| dir.join(format!("{:x}.json", sha2::Sha256::digest(&bytes))));
    if let Some(path) = &cache
        && let Ok(file) = fs::File::open(path)
        && let Ok(value) = serde_json::from_reader::<_, Value>(file.take(4096))
        && let Some(mut attachment) = Attachment::from_value(&value)
        && preview_path_for(&attachment).is_some()
        && read(&attachment).is_ok_and(|stored| stored == bytes)
    {
        attachment.name = name;
        return Ok(attachment);
    }
    let (validated, decoded) = validate(&bytes).map_err(|error| format!("{name}: {error}"))?;
    let bytes_len = bytes.len() as u64;
    let id = store_blob(&bytes, validated.format)?;
    store_preview(&id, decoded)?;
    let attachment = Attachment {
        id,
        name,
        mime: mime(validated.format).into(),
        width: validated.width,
        height: validated.height,
        bytes: bytes_len,
        path: None,
        asset_id: None,
        uploaded: None,
        source: None,
    };
    if let Some(cache) = cache {
        // Cache metadata is optional. The validated blob remains the source of truth.
        let _ = fs::write(cache, attachment.to_value().to_string());
    }
    Ok(attachment)
}

/// Network uploads specify bytes, never a destination path. Validate before atomic promotion.
pub fn store_uploaded(directory: &Path, name: &str, bytes: &[u8]) -> Result<Attachment, String> {
    use sha2::Digest;
    let (validated, _) = validate(bytes)?;
    let extension = match validated.format {
        ImageFormat::Png => "png",
        ImageFormat::Jpeg => "jpg",
        ImageFormat::Gif => "gif",
        ImageFormat::WebP => "webp",
        _ => return Err("Unsupported image format.".into()),
    };
    let id = format!("{:x}.{extension}", sha2::Sha256::digest(bytes));
    fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    let path = directory.join(&id);
    if !path.exists() {
        use std::io::Write;
        let staged = directory.join(format!("{}.upload", uuid::Uuid::new_v4()));
        let promoted = (|| {
            let mut file = fs::File::create_new(&staged)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            match fs::rename(&staged, &path) {
                Ok(()) => Ok(()),
                Err(_) if fs::read(&path).is_ok_and(|existing| existing == bytes) => Ok(()),
                Err(error) => Err(error),
            }
        })();
        let _ = fs::remove_file(&staged);
        promoted.map_err(|e: std::io::Error| e.to_string())?;
    }
    Ok(Attachment {
        id,
        name: clean_name(name),
        mime: mime(validated.format).into(),
        width: validated.width,
        height: validated.height,
        bytes: bytes.len() as u64,
        path: None,
        asset_id: None,
        uploaded: None,
        source: None,
    })
}

/// Import an image part from Pi's saved conversation history.
///
/// The returned reference points at the same local store, so later reads of history keep their
/// images without Shina's JSON retaining Pi's base64 copy.
pub fn from_history_part(part: &Value) -> Option<Attachment> {
    if let Some(value) = part.get("shinaAttachment") {
        return Attachment::from_value(value);
    }
    let data = part["data"].as_str()?;
    if data.len() as u64 > MAX_IMAGE_BYTES.div_ceil(3) * 4 + 128 {
        return None;
    }
    let bytes = decode_base64(data)?;
    let name = part["name"]
        .as_str()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("Image from conversation");
    attach_bytes(name, bytes).ok()
}

/// Called by the RPC reader, before the record enters the UI event queue.
pub fn import_history(record: &mut Value) {
    for key in ["message", "result", "partialResult"] {
        if let Some(content) = record.get_mut(key).and_then(|m| m.get_mut("content")) {
            import_parts(content);
        }
    }
    if let Some(entries) = record
        .get_mut("data")
        .and_then(|d| d.get_mut("entries"))
        .and_then(Value::as_array_mut)
    {
        for entry in entries {
            if let Some(content) = entry.get_mut("message").and_then(|m| m.get_mut("content")) {
                import_parts(content);
            }
            if entry["type"] == "custom_message"
                && let Some(content) = entry.get_mut("content")
            {
                import_parts(content);
            }
        }
    }
    if let Some(messages) = record
        .get_mut("data")
        .and_then(|d| d.get_mut("messages"))
        .and_then(Value::as_array_mut)
    {
        for message in messages {
            if let Some(content) = message.get_mut("content") {
                import_parts(content);
            }
        }
    }
}

fn import_parts(content: &mut Value) {
    if let Some(parts) = content.as_array_mut() {
        for part in parts {
            if part["type"] != "image" {
                continue;
            }
            let attachment = from_history_part(part);
            if let Some(object) = part.as_object_mut() {
                object.remove("data");
                object.insert(
                    "shinaAttachment".into(),
                    attachment.map_or(Value::Null, |image| image.to_value()),
                );
            }
        }
    }
}

/// Encode the typed Pi RPC `images` field. Call only when a prompt is submitted.
pub fn encode_images(images: &[Attachment]) -> Result<Vec<Value>, String> {
    validate_prompt_images(images)?;
    let mut encoded = Vec::with_capacity(images.len());
    for image in images.iter().filter(|a| a.is_image()) {
        let bytes = read(image)?;
        encoded.push(json!({
            "type": "image",
            "data": encode_base64(&bytes),
            "mimeType": image.mime,
        }));
    }
    Ok(encoded)
}

pub fn validate_prompt_images(images: &[Attachment]) -> Result<(), String> {
    if images.len() > MAX_ATTACHMENTS {
        return Err(format!(
            "You can attach up to {MAX_ATTACHMENTS} files per message or question response."
        ));
    }
    if images
        .iter()
        .filter(|a| {
            a.is_image()
                || matches!(
                    a.mime.to_ascii_lowercase().as_str(),
                    "image/gif" | "image/jpeg" | "image/png" | "image/webp"
                )
        })
        .map(|image| image.bytes)
        .fold(0u64, u64::saturating_add)
        > MAX_TOTAL_BYTES
    {
        return Err("Images can total up to 80 MiB per message or question response. Use smaller images or send fewer at once.".into());
    }
    for attachment in images.iter().filter(|a| a.is_image()) {
        if attachment.bytes > MAX_IMAGE_BYTES {
            return Err(crate::attachment_intake::Failure::TooLarge.message(&attachment.name));
        }
    }
    for attachment in images.iter().filter(|a| !a.is_image()) {
        if attachment.uploaded.is_some() {
            continue;
        }
        let path = attachment.path.as_ref().unwrap();
        let metadata = fs::metadata(path).map_err(|e| {
            format!(
                "Missing file {}. Remove it or restore it before sending: {e}",
                path.display()
            )
        })?;
        if !(metadata.is_file() || metadata.is_dir())
            || metadata.is_dir() != (attachment.mime == "inode/directory")
        {
            return Err(format!(
                "{} is no longer the attached file or folder.",
                path.display()
            ));
        }
    }
    Ok(())
}

pub fn provider_text(prompt: &Prompt) -> String {
    let local_id = directory()
        .and_then(|d| d.parent().map(Path::to_path_buf))
        .and_then(|d| std::fs::read_to_string(d.join("server-id")).ok())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());
    crate::composer_context::project_composer_context_for_provider_on(
        &prompt.text,
        prompt
            .context
            .as_ref()
            .map_or(&[], |c| c.records.as_slice()),
        local_id.as_deref(),
    )
}

/// Exact KSG provider-core/attachmentPrompt format. Pi does not inline ordinary text files.
pub fn pi_message(prompt: &Prompt) -> Result<String, String> {
    validate_prompt_images(&prompt.images)?;
    let mut text = provider_text(prompt);
    for attachment in prompt.images.iter() {
        let path = path_for(attachment).ok_or("Invalid attachment path.")?;
        if !text.is_empty() {
            text.push_str("\n\n");
        }
        text.push_str(&format!(
            "[Attached {} \"{}\" is saved at: {}]",
            if attachment.is_image() {
                "image"
            } else {
                "file"
            },
            attachment.name,
            path.display()
        ));
    }
    Ok(text)
}

/// Internal transport record. ACP chooses its content blocks after capability negotiation.
pub fn prompt_record(prompt: &Prompt, pi: bool) -> Result<Value, String> {
    validate_prompt_images(&prompt.images)?;
    let mut record =
        json!({"message": if pi { pi_message(prompt)? } else { provider_text(prompt) }});
    let images = encode_images(&prompt.images)?;
    if !images.is_empty() {
        record["images"] = json!(images);
    }
    if !pi {
        // Internal native-provider record, not a peer protocol field. Claude keeps paths
        // available when its retained native-image budget is exhausted.
        record["imageAttachments"] = json!(
            prompt
                .images
                .iter()
                .filter(|a| a.is_image())
                .map(Attachment::to_value)
                .collect::<Vec<_>>()
        );
        record["attachmentPaths"] = json!(
            prompt
                .images
                .iter()
                .map(Attachment::to_value)
                .collect::<Vec<_>>()
        );
        record["attachments"] = json!(
            prompt
                .images
                .iter()
                .filter(|a| !a.is_image())
                .map(Attachment::to_value)
                .collect::<Vec<_>>()
        );
    }
    Ok(record)
}

pub fn acp_parts(attachments: &[Attachment], capabilities: &Value) -> Result<Vec<Value>, String> {
    validate_prompt_images(attachments)?;
    let mut parts = Vec::new();
    for attachment in attachments.iter().filter(|a| !a.is_image()) {
        let path = attachment.path.as_ref().unwrap();
        let uri = file_uri(path);
        let text = if attachment.mime != "inode/directory" {
            text_content(path)?
        } else {
            None
        };
        if capabilities["embeddedContext"] == true
            && let Some(text) = text
        {
            parts.push(json!({"type":"resource","resource":{"uri":uri,"mimeType":"text/plain","text":text}}));
        } else if capabilities["embeddedContext"] == true {
            parts.push(json!({"type":"resource_link","uri":uri,"name":attachment.name,"mimeType":attachment.mime}));
        } else {
            parts.push(json!({"type":"text","text":format!("[Attached file \"{}\" is saved at: {}]", attachment.name, path.display())}));
        }
    }
    Ok(parts)
}

fn file_uri(path: &Path) -> String {
    let raw = path.to_string_lossy().replace('\\', "/");
    let raw = raw
        .strip_prefix("//?/UNC/")
        .map(|s| format!("//{s}"))
        .unwrap_or_else(|| raw.strip_prefix("//?/").unwrap_or(&raw).to_owned());
    let mut uri = if raw.starts_with("//") {
        "file:".to_owned()
    } else if raw.starts_with('/') {
        "file://".to_owned()
    } else {
        "file:///".to_owned()
    };
    for byte in raw.bytes() {
        if byte.is_ascii_alphanumeric() || b"/:-._~".contains(&byte) {
            uri.push(byte as char);
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri
}

pub fn read(attachment: &Attachment) -> Result<Vec<u8>, String> {
    let path = path_for(attachment).ok_or_else(|| {
        "That image reference is not valid. Remove it and attach it again.".to_owned()
    })?;
    let file = fs::File::open(&path).map_err(|error| {
        format!(
            "Could not read the saved image {}. Attach it again: {error}",
            attachment.name
        )
    })?;
    let mut bytes = Vec::new();
    file.take(MAX_IMAGE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            format!(
                "Could not read the saved image {}: {error}",
                attachment.name
            )
        })?;
    if bytes.len() as u64 != attachment.bytes || bytes.len() as u64 > MAX_IMAGE_BYTES {
        return Err(format!(
            "The saved image {} is incomplete. Attach it again.",
            attachment.name
        ));
    }
    Ok(bytes)
}

#[derive(Clone, Copy, Debug)]
struct Validated {
    format: ImageFormat,
    width: u32,
    height: u32,
}

fn validate(bytes: &[u8]) -> Result<(Validated, image::DynamicImage), String> {
    let size = bytes.len() as u64;
    if size == 0 {
        return Err("The file is empty.".into());
    }
    if size > MAX_IMAGE_BYTES {
        return Err(format!(
            "The image is {:.1} MB. Choose one that is {} MB or smaller.",
            size as f64 / (1024.0 * 1024.0),
            MAX_IMAGE_BYTES / (1024 * 1024)
        ));
    }
    let format = image::guess_format(bytes)
        .map_err(|_| "The file is not a PNG, JPEG, GIF, or WebP image.".to_owned())?;
    if !matches!(
        format,
        ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::Gif | ImageFormat::WebP
    ) {
        return Err("The file is not a PNG, JPEG, GIF, or WebP image.".into());
    }
    let (width, height) = ImageReader::with_format(Cursor::new(bytes), format)
        .into_dimensions()
        .map_err(|_| "The image is damaged or incomplete. Choose a valid image.".to_owned())?;
    if width == 0 || height == 0 {
        return Err("The image has invalid dimensions.".into());
    }
    // A header alone can look valid while the rest of the file is truncated. Decode once here,
    // off the prompt path, so malformed images fail before they reach Pi.
    let decoded = image::load_from_memory_with_format(bytes, format)
        .map_err(|_| "The image is damaged or incomplete. Choose a valid image.".to_owned())?;
    Ok((
        Validated {
            format,
            width,
            height,
        },
        decoded,
    ))
}

/// Rune decodes raster files at full resolution, regardless of their display size.
/// Store a small display copy so a transcript of large screenshots cannot fill RAM.
fn store_preview(id: &str, decoded: image::DynamicImage) -> Result<(), String> {
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    let path = path_for_id(&format!("preview-{id}.png"))
        .ok_or("Could not find Shina's image preview folder.")?;
    if path.is_file() {
        return Ok(());
    }
    let thumbnail = decoded.thumbnail(decoded.width().min(512), decoded.height().min(512));
    let mut bytes = Cursor::new(Vec::new());
    thumbnail
        .write_to(&mut bytes, ImageFormat::Png)
        .map_err(|error| format!("Could not prepare the image preview: {error}"))?;
    let temporary = path.with_extension(format!(
        "{}-{}.tmp",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ));
    let result =
        fs::write(&temporary, bytes.into_inner()).and_then(|_| fs::rename(&temporary, &path));
    if let Err(error) = result {
        let _ = fs::remove_file(&temporary);
        return Err(format!("Could not save the image preview: {error}"));
    }
    Ok(())
}

fn mime(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Jpeg => "image/jpeg",
        ImageFormat::Gif => "image/gif",
        ImageFormat::WebP => "image/webp",
        _ => "image/png",
    }
}

fn extension(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Jpeg => "jpg",
        ImageFormat::Gif => "gif",
        ImageFormat::WebP => "webp",
        _ => "png",
    }
}

fn clean_name(name: &str) -> String {
    let name = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("Image")
        .chars()
        .filter(|character| !character.is_control())
        .take(NAME_LIMIT)
        .collect::<String>();
    if name.trim().is_empty() {
        "Image".into()
    } else {
        name
    }
}

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

fn store_blob(bytes: &[u8], format: ImageFormat) -> Result<String, String> {
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    let directory = directory().ok_or_else(|| {
        "Shina's local data folder is unavailable. The image was not attached.".to_owned()
    })?;
    fs::create_dir_all(&directory)
        .map_err(|error| format!("Shina could not create its attachment folder: {error}"))?;
    let stem = format!("{:016x}-{}", fnv(bytes), bytes.len());
    let extension = extension(format);
    for suffix in 0..1000 {
        let id = if suffix == 0 {
            format!("{stem}.{extension}")
        } else {
            format!("{stem}-{suffix}.{extension}")
        };
        let path = directory.join(&id);
        match fs::read(&path) {
            Ok(existing) if existing == bytes => return Ok(id),
            Ok(_) => continue,
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                return Err(format!(
                    "Shina could not check its attachment store: {error}"
                ));
            }
            Err(_) => {}
        }
        let temporary = directory.join(format!(
            ".{stem}-{}-{}.tmp",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        if let Err(error) = fs::write(&temporary, bytes) {
            let _ = fs::remove_file(&temporary);
            return Err(format!(
                "Shina could not save the image in its local data folder: {error}"
            ));
        }
        match fs::rename(&temporary, &path) {
            Ok(()) => return Ok(id),
            Err(error) if path.is_file() => {
                let _ = fs::remove_file(&temporary);
                if fs::read(&path).is_ok_and(|existing| existing == bytes) {
                    return Ok(id);
                }
                if error.kind() != std::io::ErrorKind::AlreadyExists {
                    continue;
                }
            }
            Err(error) => {
                let _ = fs::remove_file(&temporary);
                return Err(format!(
                    "Shina could not save the image in its local data folder: {error}"
                ));
            }
        }
    }
    Err("Shina could not find a free name for the stored image.".into())
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn encode_base64(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        encoded.push(BASE64[(first >> 2) as usize] as char);
        encoded.push(BASE64[(((first & 0x03) << 4) | (second >> 4)) as usize] as char);
        encoded.push(if chunk.len() > 1 {
            BASE64[(((second & 0x0f) << 2) | (third >> 6)) as usize] as char
        } else {
            '='
        });
        encoded.push(if chunk.len() > 2 {
            BASE64[(third & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    encoded
}

pub fn decode_base64(text: &str) -> Option<Vec<u8>> {
    let text = text.trim();
    let text = if text.starts_with("data:") {
        let (header, body) = text.split_once(',')?;
        if !header.ends_with(";base64") {
            return None;
        }
        body
    } else {
        text
    };
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return None;
    }
    let mut decoded = Vec::with_capacity(bytes.len() / 4 * 3);
    for (index, chunk) in bytes.chunks(4).enumerate() {
        let last = index + 1 == bytes.len() / 4;
        let value = |byte: u8| -> Option<u8> {
            match byte {
                b'A'..=b'Z' => Some(byte - b'A'),
                b'a'..=b'z' => Some(byte - b'a' + 26),
                b'0'..=b'9' => Some(byte - b'0' + 52),
                b'+' => Some(62),
                b'/' => Some(63),
                _ => None,
            }
        };
        let padding = usize::from(chunk[3] == b'=') + usize::from(chunk[2] == b'=');
        if padding > 0 && !last
            || chunk[2] == b'=' && chunk[3] != b'='
            || chunk[..4 - padding]
                .iter()
                .any(|byte| value(*byte).is_none())
        {
            return None;
        }
        let a = u32::from(value(chunk[0])?);
        let b = if chunk[1] == b'=' {
            return None;
        } else {
            u32::from(value(chunk[1])?)
        };
        let c = if chunk[2] == b'=' {
            0
        } else {
            u32::from(value(chunk[2])?)
        };
        let d = if chunk[3] == b'=' {
            0
        } else {
            u32::from(value(chunk[3])?)
        };
        let value = (a << 18) | (b << 12) | (c << 6) | d;
        decoded.push((value >> 16) as u8);
        if padding < 2 {
            decoded.push((value >> 8) as u8);
        }
        if padding < 1 {
            decoded.push(value as u8);
        }
    }
    Some(decoded)
}

#[cfg(test)]
mod tests {
    #[test]
    fn reattach_matching_uses_ksg_metadata_not_content_or_case_folding() {
        let directory = crate::store::directory().unwrap();
        let path = directory.join("clip.mp4");
        std::fs::write(&path, b"video").unwrap();
        let mut marker = super::attach_file(&path).unwrap();
        assert!(!marker.needs_reattach());
        std::fs::remove_file(&path).unwrap();
        assert!(marker.needs_reattach());
        marker.mime = "application/octet-stream".into();
        assert!(marker.matches_reattach("clip.mp4", "video/mp4", 5));
        assert!(!marker.matches_reattach("Clip.mp4", "video/mp4", 5));
        assert!(!marker.matches_reattach("clip.mp4", "video/mp4", 6));
        marker.name = "raw.dng".into();
        marker.mime = "image/dng".into();
        assert!(marker.matches_reattach("raw.dng", "image/x-adobe-dng", 5));
        assert!(!marker.matches_reattach("raw.dng", "application/pdf", 6));
    }

    use super::*;

    #[test]
    fn ksg_image_count_and_byte_boundaries() {
        let image = Attachment {
            id: "fixture.png".into(),
            name: "fixture.png".into(),
            mime: "image/png".into(),
            width: 1,
            height: 1,
            bytes: 1,
            path: None,
            asset_id: None,
            uploaded: None,
            source: None,
        };
        assert!(validate_prompt_images(&vec![image.clone(); 100]).is_ok());
        assert_eq!(
            validate_prompt_images(&vec![image.clone(); 101]).unwrap_err(),
            "You can attach up to 100 files per message or question response."
        );
        let mut images = vec![image; 8];
        for image in &mut images {
            image.bytes = 10 * 1024 * 1024;
        }
        assert!(validate_prompt_images(&images).is_ok());
        images.push(Attachment {
            bytes: 1,
            ..images[0].clone()
        });
        assert_eq!(
            validate_prompt_images(&images).unwrap_err(),
            "Images can total up to 80 MiB per message or question response. Use smaller images or send fewer at once."
        );
    }

    #[test]
    fn supported_image_mime_files_share_the_image_budget_but_dng_does_not() {
        let path = crate::store::directory().unwrap().join("budget.bin");
        fs::write(&path, b"file").unwrap();
        let mut file = attach_file(&path).unwrap();
        file.bytes = 40 * 1024 * 1024;
        file.mime = "image/png".into();
        assert!(validate_prompt_images(&[file.clone(), file.clone()]).is_ok());
        assert!(validate_prompt_images(&[file.clone(), file.clone(), file.clone()]).is_err());
        file.mime = "image/x-adobe-dng".into();
        assert!(validate_prompt_images(&[file.clone(), file.clone(), file]).is_ok());
    }

    #[test]
    fn transferred_files_are_content_addressed_immutable_and_validate_ksg_boundaries() {
        let directory = crate::store::directory()
            .unwrap()
            .join("file-transfer-store");
        let bytes = b"\0ordinary binary\xff";
        let first = store_uploaded_file(&directory, "notes.bin", "application/octet-stream", bytes)
            .unwrap();
        let again =
            store_uploaded_file(&directory, "renamed.dat", "application/octet-stream", bytes)
                .unwrap();
        assert_eq!(first.id, again.id);
        assert!(safe_file_id(&first.id));
        assert!(!first.is_image());
        assert_eq!(transfer_file_bytes(&first).unwrap(), bytes);
        assert_eq!(
            Attachment::from_value(&first.to_value()),
            Some(first.clone())
        );
        assert_ne!(
            first.id,
            store_uploaded_file(
                &directory,
                "other.bin",
                "application/octet-stream",
                b"changed"
            )
            .unwrap()
            .id
        );
        assert!(validate_file_metadata("ok", "application/octet-stream", MAX_FILE_BYTES).is_ok());
        assert_eq!(
            validate_file_metadata("large.bin", "application/octet-stream", MAX_FILE_BYTES + 1)
                .unwrap_err(),
            "'large.bin' exceeds the 50 MB attachment limit."
        );
        assert_eq!(
            store_uploaded_file(&directory, "empty.txt", "text/plain", b"").unwrap_err(),
            "'empty.txt' is empty or could not be read."
        );
        assert!(validate_file_metadata(&"n".repeat(256), "text/plain", 1).is_err());
        assert!(validate_file_metadata("name", &"m".repeat(101), 1).is_err());
        for invalid in [
            "../file-a.bin",
            "file-a.bin",
            "file-foo.png",
            "file-000000000000000000000000000000000000000000000000000000000000000G.bin",
        ] {
            assert!(!safe_file_id(invalid), "{invalid}");
        }
        fs::write(first.path.as_ref().unwrap(), b"tampered").unwrap();
        assert_eq!(
            transfer_file_bytes(&first).unwrap_err(),
            "File hash mismatch."
        );
        assert_eq!(
            store_uploaded_file(&directory, "notes.bin", "application/octet-stream", bytes)
                .unwrap_err(),
            "Attachment already uploaded."
        );
    }

    #[test]
    fn staged_prompt_files_rewrite_context_and_snapshot_but_not_reference_ids_or_source_draft() {
        let directory = crate::store::directory().unwrap();
        let path = directory.join("immutable-send.txt");
        fs::write(&path, b"original").unwrap();
        let file = attach_file(&path).unwrap();
        let record = crate::composer_context::attachment_context_record(&file);
        let text = crate::composer_context::format_composer_context_reference(
            "file",
            &record.context_id,
            "immutable-send.txt",
        );
        let mut editor = crate::composer_rich::draft_from_canonical(&text);
        editor.set_caret(editor.text.len());
        let mut prompt = Prompt::with_context(
            text.clone(),
            vec![file.clone()],
            Some(crate::composer_context::MessageContext {
                version: 1,
                records: vec![record.clone()],
            }),
        );
        prompt.draft = Some(crate::composer_rich::Snapshot::capture_with_context(
            &editor,
            &[file.clone()],
            &[record],
        ));
        stage_prompt_files(&mut prompt).unwrap();
        let staged = &prompt.images[0];
        assert!(safe_file_id(&staged.id));
        assert_ne!(staged.id, file.id);
        assert_eq!(prompt.text, text);
        assert_eq!(
            crate::composer_rich::markdown(&prompt.draft.as_ref().unwrap().editor()),
            text
        );
        assert_eq!(
            prompt.context.as_ref().unwrap().records[0].context_id,
            crate::composer_context::to_kind_scoped_composer_context_id("file", &file.id)
        );
        let crate::composer_context::ComposerContextData::File { attachment_id, .. } =
            &prompt.context.as_ref().unwrap().records[0].data
        else {
            panic!("file")
        };
        assert_eq!(attachment_id, &staged.id);
        assert_eq!(prompt.draft.as_ref().unwrap().attachments[0], *staged);
        assert_eq!(
            prompt.draft.as_ref().unwrap().context,
            prompt.context.as_ref().unwrap().records
        );
        fs::write(path, b"new source contents").unwrap();
        assert_eq!(transfer_file_bytes(staged).unwrap(), b"original");
    }

    #[test]
    fn ordinary_file_classification_uses_content_and_retains_missing_references() {
        let directory = crate::store::directory().unwrap();
        for (name, bytes, expected) in [
            (
                "text.bin",
                b"valid UTF-8 \xc3\xa9".as_slice(),
                "application/octet-stream",
            ),
            ("binary.txt", b"hello\0world".as_slice(), "text/plain"),
            ("invalid.txt", b"\xff\xfe".as_slice(), "text/plain"),
        ] {
            let path = directory.join(name);
            fs::write(&path, bytes).unwrap();
            let attached = attach_file(&path).unwrap();
            assert!(!attached.is_image());
            assert_eq!(attached.mime, expected);
            assert!(attached.path.as_ref().unwrap().is_absolute());
            assert_eq!(
                Attachment::from_value(&attached.to_value()),
                Some(attached.clone())
            );
            fs::remove_file(&path).unwrap();
            let restored = Attachment::from_value(&attached.to_value()).unwrap();
            assert!(
                validate_prompt_images(&[restored])
                    .unwrap_err()
                    .contains("Missing file")
            );
            assert!(attach_file(&path).unwrap_err().contains("Missing file"));
        }
        let fake_image = directory.join("fake.png");
        fs::write(&fake_image, b"actually text").unwrap();
        assert_eq!(
            attach_file(&fake_image).unwrap_err(),
            "'fake.png' could not be read as an image."
        );
        assert_eq!(reference_file(&fake_image).unwrap().mime, "text/plain");
        let path = directory.join("large.txt");
        fs::write(&path, vec![b'a'; MAX_TEXT_BYTES as usize + 1]).unwrap();
        assert_eq!(attach_file(&path).unwrap().mime, "text/plain");
        fs::write(&path, vec![b'a'; MAX_TEXT_BYTES as usize]).unwrap();
        assert_eq!(attach_file(&path).unwrap().mime, "text/plain");
        assert_eq!(attach_file(&directory).unwrap().mime, "inode/directory");
    }

    #[test]
    fn pi_and_acp_prompt_parts_match_their_negotiated_protocols() {
        let directory = crate::store::directory().unwrap();
        let text_path = directory.join("note #1.txt");
        fs::write(&text_path, "Small text, not inline in Pi.").unwrap();
        let file = attach_file(&text_path).unwrap();
        let binary_path = directory.join("binary.bin");
        fs::write(&binary_path, [0, 255]).unwrap();
        let binary = attach_file(&binary_path).unwrap();
        let folder = attach_file(&directory).unwrap();
        let image = attach_bytes("image.png", image_bytes(ImageFormat::Png)).unwrap();
        let prompt = Prompt::new(
            "Read these".into(),
            vec![file.clone(), binary.clone(), folder.clone(), image],
        );
        let pi = prompt_record(&prompt, true).unwrap();
        assert_eq!(
            pi["message"],
            format!(
                "Read these\n\n[Attached file \"{}\" is saved at: {}]\n\n[Attached file \"{}\" is saved at: {}]\n\n[Attached file \"{}\" is saved at: {}]\n\n[Attached image \"image.png\" is saved at: {}]",
                file.name,
                file.path.as_ref().unwrap().display(),
                binary.name,
                binary.path.as_ref().unwrap().display(),
                folder.name,
                folder.path.as_ref().unwrap().display(),
                path_for(&prompt.images[3]).unwrap().display()
            )
        );
        assert!(!pi["message"].as_str().unwrap().contains("Small text"));
        assert_eq!(pi["images"].as_array().unwrap().len(), 1);
        assert_eq!(pi["images"][0]["type"], "image");
        assert!(pi.get("attachments").is_none());
        let acp = prompt_record(&prompt, false).unwrap();
        assert_eq!(acp["message"], "Read these");
        assert_eq!(acp["attachments"].as_array().unwrap().len(), 3);
        let parts = acp_parts(&prompt.images, &json!({"embeddedContext":true})).unwrap();
        assert_eq!(parts[0]["type"], "resource");
        assert_eq!(
            parts[0]["resource"]["text"],
            "Small text, not inline in Pi."
        );
        assert!(
            parts[0]["resource"]["uri"]
                .as_str()
                .unwrap()
                .ends_with("note%20%231.txt")
        );
        assert_eq!(parts[1]["type"], "resource_link");
        assert_eq!(parts[2]["type"], "resource_link");
        let fallback = acp_parts(&prompt.images, &json!({})).unwrap();
        assert!(fallback.iter().all(|p| p["type"] == "text"));
        assert!(
            fallback[0]["text"]
                .as_str()
                .unwrap()
                .contains("[Attached file")
        );
        // A text file changed after intake must not be embedded as text.
        fs::write(&text_path, [0, 255]).unwrap();
        assert_eq!(
            acp_parts(&[file], &json!({"embeddedContext":true})).unwrap()[0]["type"],
            "resource_link"
        );
    }

    #[test]
    fn file_count_limit_is_separate_from_image_caps() {
        let directory = crate::store::directory().unwrap();
        let folder = attach_file(&directory).unwrap();
        let attachments = vec![folder; MAX_ATTACHMENTS];
        assert!(validate_prompt_images(&attachments).is_ok());
        assert!(encode_images(&attachments).unwrap().is_empty());
        let mut overflow = attachments;
        overflow.push(overflow[0].clone());
        assert!(
            validate_prompt_images(&overflow)
                .unwrap_err()
                .contains("100 files")
        );
    }

    #[test]
    fn image_import_accepts_string_notifications_and_mixed_rpc_shapes_without_mutating_them() {
        for mut record in [
            json!({"type":"extension_ui_request","method":"confirm","message":"Trust this workspace?"}),
            json!({"type":"extension_ui_request","method":"notify","message":"Done"}),
            json!({"type":"response","command":"get_state","data":{"sessionFile":"saved.jsonl"}}),
            json!({"type":"tool_execution_end","result":{"content":[{"type":"text","text":"Output"}]}}),
        ] {
            let before = record.clone();
            import_history(&mut record);
            assert_eq!(record, before);
        }
    }
    use std::io::Cursor;

    fn image_bytes(format: ImageFormat) -> Vec<u8> {
        let mut output = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            3,
            2,
            image::Rgba([20, 40, 60, 255]),
        ))
        .write_to(&mut output, format)
        .unwrap();
        output.into_inner()
    }

    #[test]
    fn base64_round_trips_every_remainder_and_rejects_bad_input() {
        assert_eq!(encode_base64(b""), "");
        assert_eq!(encode_base64(b"f"), "Zg==");
        assert_eq!(encode_base64(b"fo"), "Zm8=");
        assert_eq!(encode_base64(b"foo"), "Zm9v");
        for bytes in [&b""[..], b"f", b"fo", b"foo", b"foob", b"fooba"] {
            assert_eq!(decode_base64(&encode_base64(bytes)).as_deref(), Some(bytes));
        }
        assert_eq!(
            decode_base64("data:image/png;base64,Zm9v").as_deref(),
            Some(&b"foo"[..])
        );
        assert!(decode_base64("Zm9").is_none());
        assert!(decode_base64("Zm=9").is_none());
        assert!(decode_base64("Zg==Zg==").is_none());
        assert!(decode_base64("!!!!").is_none());
    }

    #[test]
    fn history_import_replaces_base64_with_reusable_validated_references() {
        let png = image_bytes(ImageFormat::Png);
        let mut record = json!({"command":"get_messages","data":{"messages":[{
            "role":"user","content":[{"type":"image","data":encode_base64(&png)}]
        }]}});
        import_history(&mut record);
        let part = &record["data"]["messages"][0]["content"][0];
        assert!(part.get("data").is_none());
        let image = from_history_part(part).unwrap();
        assert_eq!(read(&image).unwrap(), png);
        let reused = attach_bytes("renamed.png", png).unwrap();
        assert_eq!(reused.id, image.id);
        assert_eq!(reused.name, "renamed.png");
    }

    #[test]
    fn validation_uses_image_content_not_the_file_extension() {
        let jpeg = image_bytes(ImageFormat::Jpeg);
        let attachment = attach_bytes("misleading.png", jpeg).unwrap();
        assert_eq!(attachment.mime, "image/jpeg");
        assert_eq!((attachment.width, attachment.height), (3, 2));
        assert!(attachment.id.ends_with(".jpg"));
        assert_eq!(read(&attachment).unwrap(), image_bytes(ImageFormat::Jpeg));
    }

    #[test]
    fn malformed_oversize_and_empty_images_fail_before_storage() {
        assert!(
            attach_bytes("not.png", b"only text".to_vec())
                .unwrap_err()
                .contains("not a PNG")
        );
        let png = image_bytes(ImageFormat::Png);
        assert!(
            attach_bytes("cut.png", png[..20].to_vec())
                .unwrap_err()
                .contains("damaged")
        );
        assert!(
            attach_bytes("empty.png", Vec::new())
                .unwrap_err()
                .contains("empty")
        );
        assert!(
            attach_bytes("huge.png", vec![0; MAX_IMAGE_BYTES as usize + 1])
                .unwrap_err()
                .contains("10 MB")
        );
    }

    #[test]
    fn file_reads_keep_large_binary_sources_and_directories_as_references() {
        let directory = crate::store::directory().unwrap();
        let path = directory.join("oversize.bin");
        let file = fs::File::create(&path).unwrap();
        file.set_len(MAX_IMAGE_BYTES + 1).unwrap();
        assert_eq!(attach_file(&path).unwrap().mime, "application/octet-stream");
        assert_eq!(attach_file(&directory).unwrap().mime, "inode/directory");
    }

    #[test]
    fn attachments_persist_as_small_references_and_read_back_byte_for_byte() {
        let png = image_bytes(ImageFormat::Png);
        let attachment = attach_bytes("screen.png", png.clone()).unwrap();
        let value = attachment.to_value();
        assert!(value.get("data").is_none());
        assert!(!value.to_string().contains(&encode_base64(&png)));
        let restored = Attachment::from_value(&value).unwrap();
        assert_eq!(restored, attachment);
        assert_eq!(read(&restored).unwrap(), png);
        assert_eq!(
            encode_images(std::slice::from_ref(&restored)).unwrap()[0]["mimeType"],
            "image/png"
        );
    }

    #[test]
    fn display_thumbnails_are_bounded_and_do_not_replace_the_original_payload() {
        let mut encoded = Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(1024, 640)
            .write_to(&mut encoded, ImageFormat::Png)
            .unwrap();
        let original = encoded.into_inner();
        let attached = attach_bytes("large.png", original.clone()).unwrap();
        assert_eq!((attached.width, attached.height), (1024, 640));
        let thumbnail = image::open(preview_path_for(&attached).unwrap()).unwrap();
        assert!(thumbnail.width() <= 512 && thumbnail.height() <= 512);
        assert_eq!(read(&attached).unwrap(), original);
    }
    #[test]
    fn png_jpeg_gif_and_webp_are_detected_from_bytes() {
        for (format, mime) in [
            (ImageFormat::Png, "image/png"),
            (ImageFormat::Jpeg, "image/jpeg"),
            (ImageFormat::Gif, "image/gif"),
            (ImageFormat::WebP, "image/webp"),
        ] {
            let attachment = attach_bytes("image.bin", image_bytes(format)).unwrap();
            assert_eq!(attachment.mime, mime);
        }
    }
}
