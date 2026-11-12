//! KSG-compatible HTML publishing. All paths below are owned blobs, never webview URLs.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    fs,
    io::Read,
    net::{IpAddr, Ipv4Addr},
    path::{Path, PathBuf},
};

pub const MAX_INLINE: usize = 512_000;
pub const MAX_PAGE: usize = 25 * 1024 * 1024;
pub const MAX_IMAGE: usize = 10 * 1024 * 1024;
pub const COLUMN_WIDTH: u32 = 728;
pub const MAX_CAPTURE_HEIGHT: u32 = 4000;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Blob {
    pub id: String,
    pub bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    #[serde(flatten)]
    pub blob: Blob,
    pub width: u32,
    pub height: u32,
}

/// `htmlRender` in a tool's structured data. Additional fields extend KSG's reference.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Reference {
    pub attachment_id: String,
    pub thread_id: usize,
    pub title: String,
    pub height: u32,
    pub html: Blob,
    #[serde(default)]
    pub heights: Vec<(u32, u32)>,
    #[serde(default)]
    pub light: Option<Snapshot>,
    #[serde(default)]
    pub dark: Option<Snapshot>,
}

impl Reference {
    pub fn read(value: &Value) -> Option<Self> {
        let r: Self = serde_json::from_value(value.clone()).ok()?;
        (r.valid()
            && r.title.encode_utf16().count() <= 200
            && !r.title.trim().is_empty()
            && (80..=2000).contains(&r.height))
        .then_some(r)
    }

    fn valid(&self) -> bool {
        self.attachment_id == self.html.id
            && html_id_thread(&self.html.id) == Some(self.thread_id)
            && self.html.bytes > 0
            && self.html.bytes <= MAX_PAGE as u64
            && self.heights.len() <= 24
            && self
                .heights
                .iter()
                .all(|(w, h)| *w > 0 && *w <= 10000 && *h <= 20000)
            && [("light", &self.light), ("dark", &self.dark)]
                .into_iter()
                .all(|(theme, s)| {
                    s.as_ref().is_none_or(|s| {
                        s.blob.id == snapshot_id(&self.html.id, theme)
                            && s.blob.bytes > 0
                            && s.blob.bytes <= MAX_PAGE as u64
                            && s.width > 0
                            && s.width <= 3200
                            && s.height > 0
                            && s.height <= MAX_CAPTURE_HEIGHT * 2
                    })
                })
    }

    pub fn snapshot(&self, dark: bool) -> Option<&Snapshot> {
        if dark {
            self.dark.as_ref()
        } else {
            self.light.as_ref()
        }
    }

    pub fn blobs(&self) -> impl Iterator<Item = &Blob> {
        std::iter::once(&self.html).chain(
            [&self.light, &self.dark]
                .into_iter()
                .flatten()
                .map(|s| &s.blob),
        )
    }
}

pub fn html_id_thread(id: &str) -> Option<usize> {
    let stem = id.strip_prefix("html-")?.strip_suffix(".html")?;
    let (thread, uuid) = stem.split_once('-')?;
    uuid::Uuid::parse_str(uuid).ok()?;
    thread.parse().ok()
}

pub fn safe_blob(id: &str) -> bool {
    if html_id_thread(id).is_some() {
        return true;
    }
    ["light", "dark"].iter().any(|theme| {
        id.strip_suffix(&format!("-{theme}.png"))
            .is_some_and(|base| html_id_thread(&format!("{base}.html")).is_some())
    })
}

fn snapshot_id(id: &str, theme: &str) -> String {
    format!("{}-{theme}.png", id.trim_end_matches(".html"))
}

#[derive(Debug)]
pub struct Input {
    pub html: String,
    pub title: String,
    pub height: u32,
    pub width: u32,
    pub dark: bool,
    pub missing: Vec<String>,
}

fn integer(
    args: &Value,
    key: &str,
    default: Option<u32>,
    min: u32,
    max: u32,
) -> Result<u32, String> {
    let value = match args.get(key) {
        Some(value) => value.as_u64().and_then(|n| u32::try_from(n).ok()),
        None => default,
    };
    value
        .filter(|n| (min..=max).contains(n))
        .ok_or_else(|| format!("{key} must be an integer between {min} and {max}."))
}

/// KSG's tools.ts and handlers.ts contract. Inline images obey the caller's file access too.
pub fn input(args: &Value, preview: bool, full_access: bool) -> Result<Input, String> {
    let html = match (args.get("html"), args.get("path")) {
        (Some(html), None) => {
            let html = html.as_str().ok_or("html must be a string.")?;
            if html.is_empty() || html.encode_utf16().count() > MAX_INLINE {
                return Err(format!(
                    "Inline html must contain 1 to {MAX_INLINE} characters."
                ));
            }
            html.to_owned()
        }
        (None, Some(path)) => {
            if !full_access {
                return Err(
                    "Pass html instead of path: only full-access threads may pass a path.".into(),
                );
            }
            read_page(Path::new(path.as_str().ok_or("path must be a string.")?))?
        }
        _ => return Err("Pass exactly one of html or path.".into()),
    };
    reject_viewport_height(&html)?;
    let (html, missing) = inline_images(&html, full_access)?;
    if !preview && !missing.is_empty() {
        return Err(format!(
            "These local images could not be read: {}.",
            missing.join(", ")
        ));
    }
    let (title, height, width, dark) = if preview {
        let dark = match args.get("appearance").and_then(Value::as_str) {
            None if args.get("appearance").is_none() => true,
            Some("dark") => true,
            Some("light") => false,
            _ => return Err("appearance must be dark or light.".into()),
        };
        (
            String::new(),
            800,
            integer(args, "width", Some(COLUMN_WIDTH), 240, 1600)?,
            dark,
        )
    } else {
        let title = args["title"].as_str().ok_or("title is required.")?;
        if title.is_empty() || title.encode_utf16().count() > 200 {
            return Err("title must contain 1 to 200 characters.".into());
        }
        (
            if title.trim().is_empty() {
                "HTML"
            } else {
                title.trim()
            }
            .into(),
            integer(args, "height", None, 80, 2000)?,
            COLUMN_WIDTH,
            true,
        )
    };
    Ok(Input {
        html,
        title,
        height,
        width,
        dark,
        missing,
    })
}

fn read_bounded(path: &Path, max: usize) -> Result<Vec<u8>, String> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options
        .open(path)
        .map_err(|e| format!("Could not read {}: {e}", path.display()))?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > max as u64 {
        return Err(format!(
            "{} must be a regular file of at most {max} bytes.",
            path.display()
        ));
    }
    let mut bytes = Vec::new();
    file.take(max as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > max {
        return Err(format!("{} exceeds {max} bytes.", path.display()));
    }
    Ok(bytes)
}

fn read_page(path: &Path) -> Result<String, String> {
    if !path.is_absolute()
        || path.to_string_lossy().starts_with("//")
        || path.to_string_lossy().starts_with("\\\\")
    {
        return Err("Pass an absolute path to a regular .html file.".into());
    }
    let real = path.canonicalize().map_err(|e| e.to_string())?;
    if !real
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("html") || e.eq_ignore_ascii_case("htm"))
    {
        return Err("The resolved path must have an .html or .htm extension.".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if fs::metadata(&real).map_err(|e| e.to_string())?.nlink() != 1 {
            return Err("Hard-linked HTML files are not accepted.".into());
        }
    }
    let html = String::from_utf8(read_bounded(&real, MAX_PAGE)?).map_err(|e| e.to_string())?;
    if !regex::Regex::new(r"(?i)^\s*<(?:!doctype\s+html|html)[\s>]")
        .unwrap()
        .is_match(&html)
    {
        return Err("The file must start with <!doctype html> or <html>.".into());
    }
    Ok(html)
}

/// Ported from KSG HtmlRender's CSS_SOURCE and VIEWPORT_HEIGHT_UNIT checks.
pub fn reject_viewport_height(html: &str) -> Result<(), String> {
    let source = regex::Regex::new(r#"(?is)<!--.*?(?:-->|$)|<style\b[^>]*(?:>|$)(.*?)(?:</style\s*>|$)|\sstyle\s*=\s*(?:"([^"]*)(?:"|$)|'([^']*)(?:'|$)|([^\s>]+))"#).unwrap();
    let unit =
        regex::Regex::new(r"(?i)(?:^|[^\w.+/])((?:\d+(?:\.\d+)?|\.\d+)[sld]?vh)(?:$|[^\w-])")
            .unwrap();
    for captures in source.captures_iter(html) {
        if let Some(css) = (1..=4).find_map(|i| captures.get(i))
            && unit.is_match(css.as_str())
        {
            return Err("The page uses a viewport height unit. Let content set the height or use fixed pixel heights.".into());
        }
    }
    Ok(())
}

fn image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.starts_with(b"\0\0\x01\0") {
        Some("image/x-icon")
    } else if bytes.starts_with(b"BM") && bytes.get(6..10) == Some(&[0; 4]) {
        Some("image/bmp")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some("image/webp")
    } else if matches!(
        bytes.get(4..12),
        Some(b"ftypavif" | b"ftypavis" | b"ftypmif1")
    ) {
        Some("image/avif")
    } else {
        let head = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]);
        let svg = regex::Regex::new(
            r"(?s)^\s*(?:(?:<\?.*?\?>|<!--.*?-->|<!DOCTYPE[^>]*>)\s*)*<svg[\s/>]",
        )
        .unwrap();
        svg.is_match(&head).then_some("image/svg+xml")
    }
}

fn inline_images(html: &str, full_access: bool) -> Result<(String, Vec<String>), String> {
    let paths = regex::Regex::new(r#"(?i)"((?:/[^/]|[a-z]:[\\/])[^"\r\n]*\.(?:png|jpe?g|gif|webp|avif|svg|bmp|ico))"|'((?:/[^/]|[a-z]:[\\/])[^'\r\n]*\.(?:png|jpe?g|gif|webp|avif|svg|bmp|ico))'|`((?:/[^/]|[a-z]:[\\/])[^`\r\n]*\.(?:png|jpe?g|gif|webp|avif|svg|bmp|ico))`|url\(\s*((?:/[^/]|[a-z]:[\\/])[^\s"'`()]*\.(?:png|jpe?g|gif|webp|avif|svg|bmp|ico))\s*\)"#).unwrap();
    let mut images = HashMap::new();
    let mut missing = vec![];
    let mut result = String::new();
    let mut cursor = 0;
    for captures in paths.captures_iter(html) {
        let found = (1..=4).find_map(|i| captures.get(i)).unwrap();
        let path = found.as_str();
        if path.len() > 2060 {
            return Err("A local image path exceeds 2048 characters.".into());
        }
        if !full_access {
            return Err(
                "Local image inlining requires full access. Pass data:image URLs instead.".into(),
            );
        }
        if !images.contains_key(path) {
            let file_path = if path.as_bytes().get(1) == Some(&b':') {
                path.replace("\\\\", "\\")
            } else {
                path.into()
            };
            let real_path =
                fs::canonicalize(&file_path).unwrap_or_else(|_| PathBuf::from(&file_path));
            let bytes = match read_bounded(&real_path, MAX_IMAGE) {
                Ok(bytes) => Some(bytes),
                Err(error)
                    if fs::metadata(&file_path).is_ok_and(|m| m.len() > MAX_IMAGE as u64) =>
                {
                    return Err(error);
                }
                Err(_) => None,
            };
            let data = bytes.and_then(|bytes| {
                image_mime(&bytes).map(|mime| {
                    format!(
                        "data:{mime};base64,{}",
                        crate::attachments::encode_base64(&bytes)
                    )
                })
            });
            if data.is_none() {
                missing.push(path.to_owned());
            }
            images.insert(path.to_owned(), data);
        }
        result.push_str(&html[cursor..found.start()]);
        result.push_str(images[path].as_deref().unwrap_or(path));
        if result.len() > MAX_PAGE {
            return Err("With its images inlined the page exceeds 25 MiB.".into());
        }
        cursor = found.end();
    }
    result.push_str(&html[cursor..]);
    if result.len() > MAX_PAGE {
        return Err("With its images inlined the page exceeds 25 MiB.".into());
    }
    Ok((result, missing))
}

/// WHATWG URL parsing canonicalizes decimal, hexadecimal and shortened IPv4 forms.
/// DNS answers must also pass `public_ip` immediately before connecting.
pub fn public_url(raw: &str) -> bool {
    let Ok(url) = url::Url::parse(raw) else {
        return false;
    };
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return false;
    }
    match url.host() {
        Some(url::Host::Ipv4(ip)) => public_ip(IpAddr::V4(ip)),
        Some(url::Host::Ipv6(ip)) => public_ip(IpAddr::V6(ip)),
        Some(url::Host::Domain(host)) => {
            let host = host.trim_end_matches('.').to_ascii_lowercase();
            host.contains('.')
                && ![
                    "localhost",
                    "local",
                    "internal",
                    "home",
                    "lan",
                    "test",
                    "invalid",
                    "onion",
                ]
                .iter()
                .any(|suffix| host == *suffix || host.ends_with(&format!(".{suffix}")))
        }
        _ => false,
    }
}

pub fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_unspecified()
                && !ip.is_broadcast()
                && !ip.is_multicast()
                && a != 0
                && a < 224
                && !(a == 100 && (64..=127).contains(&b))
                && !(a == 192 && b == 0 && c == 0)
                && !(a == 198 && (b == 18 || b == 19))
                && ip != Ipv4Addr::new(169, 254, 169, 254)
        }
        IpAddr::V6(ip) => {
            if let Some(v4) = ip.to_ipv4_mapped() {
                return public_ip(IpAddr::V4(v4));
            }
            // Only global unicast. Excludes loopback, compatible IPv4, ULA, link-local,
            // multicast, NAT64 and transition prefixes that could tunnel a private address.
            let s = ip.segments();
            (s[0] & 0xe000) == 0x2000 && s[0] != 0x2002 && !(s[0] == 0x2001 && s[1] == 0)
        }
    }
}

pub fn publish(
    directory: &Path,
    thread: usize,
    input: &Input,
    renderer: &dyn crate::html_window::Renderer,
) -> Result<Reference, String> {
    fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    let id = format!("html-{thread}-{}.html", uuid::Uuid::new_v4());
    let mut reference = Reference {
        attachment_id: id.clone(),
        thread_id: thread,
        title: input.title.clone(),
        height: input.height,
        html: Blob {
            id: id.clone(),
            bytes: input.html.len() as u64,
        },
        heights: vec![],
        light: None,
        dark: None,
    };
    crate::store::write_atomic(&directory.join(&id), input.html.as_bytes())
        .map_err(|e| e.to_string())?;
    for dark in [false, true] {
        if let Ok(capture) = renderer.capture(&input.html, input.width, dark) {
            let id = snapshot_id(&id, if dark { "dark" } else { "light" });
            crate::store::write_atomic(&directory.join(&id), &capture.png)
                .map_err(|e| e.to_string())?;
            if let Some((_, height)) = reference
                .heights
                .iter_mut()
                .find(|(width, _)| *width == input.width)
            {
                *height = (*height).max(capture.content_height);
            } else {
                reference
                    .heights
                    .push((input.width, capture.content_height));
            }
            let snapshot = Snapshot {
                blob: Blob {
                    id,
                    bytes: capture.png.len() as u64,
                },
                width: capture.width,
                height: capture.height,
            };
            if dark {
                reference.dark = Some(snapshot);
            } else {
                reference.light = Some(snapshot);
            }
        }
    }
    // The manifest is the publication boundary. Arbitrary HTML files are never served.
    crate::store::write_atomic(
        &manifest_path(directory, &id),
        &serde_json::to_vec(&reference).unwrap(),
    )
    .map_err(|e| e.to_string())?;
    Ok(reference)
}

fn manifest_path(directory: &Path, id: &str) -> PathBuf {
    directory.join(format!("{id}.json"))
}

pub fn owned_blob(directory: &Path, id: &str) -> bool {
    let html_id = if html_id_thread(id).is_some() {
        id.to_owned()
    } else if let Some(base) = id
        .strip_suffix("-light.png")
        .or_else(|| id.strip_suffix("-dark.png"))
    {
        format!("{base}.html")
    } else {
        return false;
    };
    html_id_thread(&html_id)
        .and_then(|thread| published(directory, &html_id, thread))
        .is_some_and(|r| r.blobs().any(|blob| blob.id == id))
}

/// Import only a complete published record. No collector removes published manifests or
/// their blobs; transcript references therefore remain valid, including archived history.
pub fn import(directory: &Path, reference: &Reference) -> Result<(), String> {
    if !reference.valid() {
        return Err("Invalid HTML render reference.".into());
    }
    for blob in reference.blobs() {
        if fs::metadata(directory.join(&blob.id))
            .map_err(|e| e.to_string())?
            .len()
            != blob.bytes
        {
            return Err("HTML render blob length mismatch.".into());
        }
    }
    crate::store::write_atomic(
        &manifest_path(directory, &reference.html.id),
        &serde_json::to_vec(reference).unwrap(),
    )
    .map_err(|e| e.to_string())
}

pub fn published(directory: &Path, id: &str, thread: usize) -> Option<Reference> {
    if html_id_thread(id) != Some(thread) {
        return None;
    }
    let r = Reference::read(
        &serde_json::from_slice::<Value>(&fs::read(manifest_path(directory, id)).ok()?).ok()?,
    )?;
    if r.thread_id != thread || r.html.id != id {
        return None;
    }
    Some(r)
}

pub fn page(directory: &Path, reference: &Reference) -> Result<String, String> {
    let owned = published(directory, &reference.attachment_id, reference.thread_id)
        .ok_or("This HTML render is not published.")?;
    if owned != *reference {
        return Err("The HTML render reference does not match its publication.".into());
    }
    String::from_utf8(read_bounded(&directory.join(&owned.html.id), MAX_PAGE)?)
        .map_err(|e| e.to_string())
}

pub fn result(reference: &Reference) -> Value {
    json!({"htmlRender":reference,"message":"Shown to the reader above your reply. Don't mention or describe the page; reply with only what it doesn't already say."})
}

/// Providers wrap MCP results differently. Accept only a validated render reference,
/// not arbitrary HTML in a tool's arguments. Bound recursion and text parsing.
pub fn find_reference(value: &Value) -> Option<Reference> {
    fn find(value: &Value, depth: usize) -> Option<Reference> {
        if depth == 0 {
            return None;
        }
        if let Some(r) = Reference::read(&value["htmlRender"]) {
            return Some(r);
        }
        match value {
            Value::Object(object) => [
                "structuredContent",
                "details",
                "codex",
                "result",
                "content",
                "output",
                "text",
            ]
            .iter()
            .filter_map(|key| object.get(*key))
            .find_map(|v| find(v, depth - 1)),
            Value::Array(values) => values.iter().take(64).find_map(|v| find(v, depth - 1)),
            Value::String(text) if text.len() <= 64 * 1024 => serde_json::from_str::<Value>(text)
                .ok()
                .and_then(|v| find(&v, depth - 1)),
            _ => value.get("text").and_then(|v| find(v, depth - 1)),
        }
    }
    find(value, 10)
}

pub fn request(
    request: crate::mcp::Request,
    directory: PathBuf,
    full_access: bool,
    lane_width: u32,
    wake: rune::Waker,
) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static ACTIVE: AtomicUsize = AtomicUsize::new(0);
    if ACTIVE.fetch_add(1, Ordering::AcqRel) >= 2 {
        ACTIVE.fetch_sub(1, Ordering::AcqRel);
        let _ = request.reply.send(Err(
            "Two HTML renders are already running. Retry after they finish.".into(),
        ));
        return;
    }
    std::thread::spawn(move || {
        struct Slot;
        impl Drop for Slot {
            fn drop(&mut self) {
                ACTIVE.fetch_sub(1, Ordering::AcqRel);
            }
        }
        let _slot = Slot;
        let preview = request.tool == "html_preview";
        let outcome = input(&request.args, preview, full_access).and_then(|mut input| {
            if !preview {
                input.width = lane_width.clamp(240, 1600);
            }
            if preview {
                crate::html_window::preview(&input, &crate::html_window::Native)
            } else {
                publish(
                    &directory,
                    request.thread,
                    &input,
                    &crate::html_window::Native,
                )
                .map(|r| result(&r))
            }
        });
        let _ = request.reply.send(outcome);
        wake();
    });
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::html_window::{Capture, Renderer};

    pub struct Fake(pub bool);
    impl Renderer for Fake {
        fn capture(&self, _html: &str, width: u32, _dark: bool) -> Result<Capture, String> {
            if !self.0 {
                return Err("No native renderer.".into());
            }
            let mut png = std::io::Cursor::new(Vec::new());
            image::RgbaImage::from_pixel(width * 2, 1600, image::Rgba([210, 100, 60, 255]))
                .write_to(&mut png, image::ImageFormat::Png)
                .unwrap();
            Ok(Capture {
                png: png.into_inner(),
                width: width * 2,
                height: 1600,
                content_height: 800,
                console: vec![],
            })
        }
    }

    pub fn fixture(snapshot: bool) -> Reference {
        let directory = crate::store::directory().unwrap().join("attachments");
        let input = input(
            &json!({"html":"<h1>Example</h1>","title":"Example chart","height":800}),
            false,
            false,
        )
        .unwrap();
        publish(&directory, 1, &input, &Fake(snapshot)).unwrap()
    }

    #[test]
    fn validates_source_exactly_once_and_every_argument_limit() {
        for source in [
            json!({}),
            json!({"html":"x","path":"/a.html"}),
            json!({"html":""}),
            json!({"html":4}),
            json!({"html":null}),
        ] {
            assert!(input(&source, true, true).is_err(), "{source}");
        }
        assert!(
            input(&json!({"path":"/missing.html"}), true, false)
                .unwrap_err()
                .contains("full-access")
        );
        for width in [239, 1601, -1] {
            assert!(input(&json!({"html":"x","width":width}), true, false).is_err());
        }
        for width in [240, 1600] {
            assert_eq!(
                input(&json!({"html":"x","width":width}), true, false)
                    .unwrap()
                    .width,
                width
            );
        }
        assert!(input(&json!({"html":"x","width":240.5}), true, false).is_err());
        assert!(input(&json!({"html":"x","appearance":"auto"}), true, false).is_err());
        assert!(input(&json!({"html":"x","appearance":null}), true, false).is_err());
        for height in [79, 2001] {
            assert!(
                input(
                    &json!({"html":"x","title":"Page","height":height}),
                    false,
                    false
                )
                .is_err()
            );
        }
        for height in [80, 2000] {
            assert!(
                input(
                    &json!({"html":"x","title":"Page","height":height}),
                    false,
                    false
                )
                .is_ok()
            );
        }
        for title in ["".to_owned(), "x".repeat(201)] {
            assert!(input(&json!({"html":"x","title":title,"height":80}), false, false).is_err());
        }
        assert!(input(&json!({"html":"x".repeat(MAX_INLINE)}), true, false).is_ok());
        assert!(input(&json!({"html":"x".repeat(MAX_INLINE+1)}), true, false).is_err());
        assert!(input(&json!({"html":"😀".repeat(MAX_INLINE/2+1)}), true, false).is_err());
    }

    #[test]
    fn rejects_height_units_in_css_but_not_prose_comments_or_base64() {
        for css in [
            "height:100vh",
            "height: 1.2svh",
            "max-height:.5dvh",
            "height:100lvh",
        ] {
            for html in [
                format!("<style>{css}</style>"),
                format!("<div style='{css}'>"),
                format!("<div style=\"{css}\">"),
                format!("<div style={}>", css.replace(' ', "")),
            ] {
                assert!(reject_viewport_height(&html).is_err(), "{html}");
            }
        }
        for html in [
            "<p>100vh is not allowed</p>",
            "<!-- <style>height:100vh</style> -->",
            "<style>background:url(data:abc/1vh)</style>",
            "<style>height:100px</style>",
        ] {
            assert!(reject_viewport_height(html).is_ok(), "{html}");
        }
    }

    #[test]
    fn files_require_absolute_regular_html_documents_and_obey_page_limit() {
        let directory = crate::store::directory().unwrap();
        let path = directory.join("page.html");
        fs::write(&path, "<!doctype html><h1>Page</h1>").unwrap();
        assert!(input(&json!({"path":path}), true, true).is_ok());
        assert!(read_page(Path::new("page.html")).is_err());
        fs::write(&path, "TOKEN=not-html").unwrap();
        assert!(read_page(&path).is_err());
        fs::File::create(&path)
            .unwrap()
            .set_len(MAX_PAGE as u64 + 1)
            .unwrap();
        assert!(read_page(&path).is_err());
        assert!(read_page(&directory).is_err());
    }

    #[test]
    fn local_images_use_magic_bytes_enforce_file_and_total_limits_and_access() {
        let directory = crate::store::directory().unwrap();
        let path = directory.join("image.png");
        fs::write(&path, b"\x89PNG\r\n\x1a\nimage bytes").unwrap();
        let html = format!("<img src=\"{}\">", path.display());
        let (inlined, missing) = inline_images(&html, true).unwrap();
        assert!(missing.is_empty());
        assert!(inlined.contains("data:image/png;base64,"));
        assert!(inline_images(&html, false).is_err());
        fs::write(&path, b"SECRET=not an image").unwrap();
        let (_, missing) = inline_images(&html, true).unwrap();
        assert_eq!(missing.len(), 1);
        assert!(
            input(
                &json!({"html":html,"title":"Page","height":80}),
                false,
                true
            )
            .is_err()
        );
        let file = fs::File::create(&path).unwrap();
        file.set_len(MAX_IMAGE as u64 + 1).unwrap();
        assert!(inline_images(&html, true).is_err());
        fs::write(&path, b"\x89PNG\r\n\x1a\n").unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(MAX_IMAGE as u64)
            .unwrap();
        assert!(inline_images(&html, true).is_ok());
        assert!(inline_images(&html.repeat(3), true).is_err());
        assert!(inline_images(&"x".repeat(MAX_PAGE + 1), true).is_err());
        assert!(!image_mime(b"<svgSECRET>foo").is_some());
        assert_eq!(
            image_mime(b"<?xml version='1.0'?><!-- x --><svg xmlns='x'>"),
            Some("image/svg+xml")
        );
    }

    #[test]
    fn public_request_filter_rejects_local_and_noncanonical_address_forms() {
        for host in [
            "localhost",
            "localhost.",
            "example.local",
            "10.0.0.1",
            "172.16.2.1",
            "192.168.1.1",
            "127.0.0.1",
            "127.1",
            "2130706433",
            "0x7f000001",
            "0177.0.0.1",
            "169.254.169.254",
            "100.64.0.1",
            "198.18.0.1",
            "0.0.0.0",
            "224.0.0.1",
            "[::1]",
            "[::]",
            "[::ffff:127.0.0.1]",
            "[::ffff:192.168.1.1]",
            "[fe80::1]",
            "[fc00::1]",
            "[fd12::1]",
            "[64:ff9b::7f00:1]",
            "[2002:7f00:1::1]",
            "[2001::1]",
        ] {
            assert!(!public_url(&format!("https://{host}/")), "{host}");
        }
        for url in [
            "file:///etc/passwd",
            "ftp://example.com/",
            "http://user:secret@example.com",
            "data:text/html,x",
            "http://[fe80::1%25en0]/",
        ] {
            assert!(!public_url(url), "{url}");
        }
        for url in [
            "https://example.com/",
            "https://8.8.8.8/",
            "https://[2606:4700:4700::1111]/",
        ] {
            assert!(public_url(url), "{url}");
        }
    }

    #[test]
    fn published_blobs_are_bound_to_thread_and_retained_across_reload() {
        let r = fixture(true);
        let directory = crate::store::directory().unwrap().join("attachments");
        assert_eq!(published(&directory, &r.attachment_id, 1), Some(r.clone()));
        assert!(published(&directory, &r.attachment_id, 2).is_none());
        for blob in r.blobs() {
            assert!(owned_blob(&directory, &blob.id));
        }
        let fake = format!("html-1-{}.html", uuid::Uuid::new_v4());
        fs::write(directory.join(&fake), "<h1>Unpublished</h1>").unwrap();
        assert!(!owned_blob(&directory, &fake));
        assert!(!safe_blob("../page.html"));
        let mut t = crate::model::Thread::new(1);
        t.apply(json!({"type":"tool_execution_start","toolCallId":"html","toolName":"html_render","args":{}}));
        t.apply(json!({"type":"tool_execution_end","toolCallId":"html","result":{"structuredContent":result(&r),"content":[]}}));
        let encoded = serde_json::to_vec(&t.tools[0]).unwrap();
        let restored: crate::model::Tool = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(find_reference(&restored.structured), Some(r.clone()));
        assert!(page(&directory, &r).is_ok());
        assert!(r.blobs().all(|blob| directory.join(&blob.id).is_file()));
        let old: crate::model::Tool = serde_json::from_value(json!({
            "id":"old","name":"read","detail":"","done":true,"failed":false,"output":"",
            "images":[],"structured":{},"parent_call":null,"exit_code":null,"subagents":[],
            "anchor":{"message":0,"offset":0},"block":0,"duration_ms":null
        }))
        .unwrap();
        assert!(find_reference(&old.structured).is_none());
    }

    #[test]
    fn capture_unavailable_still_publishes_and_preview_has_no_publication() {
        let r = fixture(false);
        assert!(r.light.is_none() && r.dark.is_none());
        let input = input(&json!({"html":"<h1>Page</h1>","width":390}), true, false).unwrap();
        let preview = crate::html_window::preview(&input, &Fake(true)).unwrap();
        assert_eq!(preview["width"], 390);
        assert_eq!(preview["contentHeight"], 800);
        assert!(preview["screenshot"]["data"].as_str().is_some());
    }

    #[test]
    #[ignore = "requires the macOS/Windows desktop and a prior cargo build"]
    fn native_offscreen_capture_is_nonblank() {
        let canary = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        canary.set_nonblocking(true).unwrap();
        let port = canary.local_addr().unwrap().port();
        let secret = crate::store::directory().unwrap().join("private.js");
        fs::write(&secret, "console.log('FILESYSTEM_LEAK')").unwrap();
        let html = format!(
            "<h1 style='color:#d2643c'>Native capture</h1><div style='width:120px;height:80px;background:#1f6d62'></div>\
            <script>console.log('captured')</script><script src='file://{}'></script>\
            <img src='http://127.0.0.1:{port}/private'><img src='http://2130706433:{port}/private'>\
            <script>fetch('http://127.0.0.1:{port}/mcp').catch(()=>{{}});let image=new Image();image.src='ht'+'tp://127.0.0.1:{port}/dynamic';let style=document.createElement('style');document.head.append(style);style.sheet.insertRule('body{{background-image:url(ht'+'tp://127.0.0.1:{port}/css)}}');</script>",
            secret.display()
        );
        let capture = crate::html_window::Native
            .capture(&html, 728, true)
            .expect("native WKWebView/WebView2 capture");
        let image = image::load_from_memory(&capture.png).unwrap().to_rgba8();
        assert_eq!(image.width(), 1456);
        let first = *image.get_pixel(0, 0);
        assert!(
            image.pixels().any(|pixel| *pixel != first),
            "snapshot must not be a uniform blank page"
        );
        assert!(capture.content_height > 80);
        assert!(
            !capture
                .console
                .iter()
                .any(|message| message.text.contains("FILESYSTEM_LEAK"))
        );
        assert!(
            matches!(canary.accept(),Err(error) if error.kind() == std::io::ErrorKind::WouldBlock),
            "untrusted HTML must never reach a loopback listener"
        );
        assert!(
            capture
                .console
                .iter()
                .any(|message| message.text.contains("captured"))
        );
    }
}
