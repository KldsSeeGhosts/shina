//! Composer image attachments.
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

/// A message carries at most this many images.
pub const MAX_IMAGES: usize = 8;
/// The largest single image Shina accepts, before base64.
pub const MAX_IMAGE_BYTES: u64 = 10 * 1024 * 1024;
/// The largest combined image payload in one draft.
pub const MAX_TOTAL_BYTES: u64 = 20 * 1024 * 1024;
/// Caps one decode at 32 megapixels, below Rune's 64 megapixel clipboard limit.
const MAX_IMAGE_PIXELS: u64 = 32 * 1024 * 1024;
const NAME_LIMIT: usize = 120;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attachment {
    /// A filename inside Shina's attachment directory, never a caller-supplied path.
    pub id: String,
    /// The name shown in a chip. Display only, never used as a path.
    pub name: String,
    pub mime: String,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
}

impl Attachment {
    pub fn to_value(&self) -> Value {
        json!({
            "id": self.id,
            "name": self.name,
            "mime": self.mime,
            "width": self.width,
            "height": self.height,
            "bytes": self.bytes,
        })
    }

    /// Restore a reference only while its immutable blob is still the expected size.
    pub fn from_value(value: &Value) -> Option<Self> {
        let id = value["id"].as_str()?;
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
        if width == 0
            || height == 0
            || u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS
            || bytes == 0
            || bytes > MAX_IMAGE_BYTES
        {
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
        })
    }
}

/// A prompt snapshot. Editing the draft after a send never changes these references.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prompt {
    pub text: String,
    pub images: Arc<[Attachment]>,
}

impl Prompt {
    pub fn new(text: String, images: Vec<Attachment>) -> Self {
        Self {
            text,
            images: images.into(),
        }
    }
}

pub fn directory() -> Option<PathBuf> {
    crate::store::directory().map(|directory| directory.join("attachments"))
}

pub fn path_for(attachment: &Attachment) -> Option<PathBuf> {
    path_for_id(&attachment.id)
}

fn path_for_id(id: &str) -> Option<PathBuf> {
    safe_id(id).then(|| directory().map(|directory| directory.join(id)))?
}

/// A blob id is one filename with a supported image extension.
fn safe_id(id: &str) -> bool {
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

/// Validate and copy an image file into Shina's local attachment store.
pub fn attach_file(path: &Path) -> Result<Attachment, String> {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Image".into());
    let file = fs::File::open(path).map_err(|error| {
        format!(
            "Could not read {}. Check that it still exists and is a file: {error}",
            clean_name(&name)
        )
    })?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("Choose an image file, not a folder or device.".into());
    }
    if metadata.len() > MAX_IMAGE_BYTES {
        return Err(format!(
            "{}: Choose an image that is {} MB or smaller.",
            clean_name(&name),
            MAX_IMAGE_BYTES / (1024 * 1024)
        ));
    }
    // Bound the read as well as checking metadata, since the file can grow while we read it.
    let mut bytes = Vec::new();
    file.take(MAX_IMAGE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Could not read {}: {error}", clean_name(&name)))?;
    attach_bytes(&name, bytes)
}

/// Validate and store image bytes, with the name shown in the composer chip.
pub fn attach_bytes(name: &str, bytes: Vec<u8>) -> Result<Attachment, String> {
    let name = clean_name(name);
    let validated = validate(&bytes).map_err(|error| format!("{name}: {error}"))?;
    let bytes_len = bytes.len() as u64;
    let id = store_blob(&bytes, validated.format)?;
    Ok(Attachment {
        id,
        name,
        mime: mime(validated.format).into(),
        width: validated.width,
        height: validated.height,
        bytes: bytes_len,
    })
}

/// Import an image part from Pi's saved conversation history.
///
/// The returned reference points at the same local store, so later reads of history keep their
/// images without Shina's JSON retaining Pi's base64 copy.
pub fn from_history_part(part: &Value) -> Option<Attachment> {
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

/// Encode the typed Pi RPC `images` field. Call only when a prompt is submitted.
pub fn encode_images(images: &[Attachment]) -> Result<Vec<Value>, String> {
    validate_prompt_images(images)?;
    let mut encoded = Vec::with_capacity(images.len());
    for image in images {
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
    if images.len() > MAX_IMAGES {
        return Err(format!(
            "This draft has {} images. Remove images until at most {MAX_IMAGES} remain.",
            images.len()
        ));
    }
    if images.iter().map(|image| image.bytes).sum::<u64>() > MAX_TOTAL_BYTES {
        return Err(format!(
            "These images exceed the {} MB message limit. Remove images before sending.",
            MAX_TOTAL_BYTES / (1024 * 1024)
        ));
    }
    Ok(())
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

fn validate(bytes: &[u8]) -> Result<Validated, String> {
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
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS {
        return Err(format!(
            "The image is too large to decode. Choose one under {} megapixels.",
            MAX_IMAGE_PIXELS / (1024 * 1024)
        ));
    }
    // A header alone can look valid while the rest of the file is truncated. Decode once here,
    // off the prompt path, so malformed images fail before they reach Pi.
    image::load_from_memory_with_format(bytes, format)
        .map_err(|_| "The image is damaged or incomplete. Choose a valid image.".to_owned())?;
    Ok(Validated {
        format,
        width,
        height,
    })
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
    use super::*;
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
    fn file_reads_reject_oversize_sources_and_directories() {
        let directory = crate::store::directory().unwrap();
        let path = directory.join("oversize.png");
        let file = fs::File::create(&path).unwrap();
        file.set_len(MAX_IMAGE_BYTES + 1).unwrap();
        assert!(attach_file(&path).unwrap_err().contains("10 MB"));
        assert!(attach_file(&directory).is_err());
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
