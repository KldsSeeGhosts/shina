//! Native HTML lives in an owned helper process, never in Rune's GPU window.
//! The pipe carries only page input, capture output, theme updates and focus requests.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{ChildStdin, Stdio},
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

#[cfg(target_os = "macos")]
#[path = "html_window_macos.rs"]
mod platform;
#[cfg(windows)]
#[path = "html_window_windows.rs"]
mod platform;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleMessage {
    pub level: String,
    pub text: String,
}

#[derive(Debug)]
pub struct Capture {
    pub png: Vec<u8>,
    /// Physical PNG pixels. CSS width and height are half of these.
    pub width: u32,
    pub height: u32,
    pub content_height: u32,
    pub console: Vec<ConsoleMessage>,
}

pub trait Renderer {
    fn capture(&self, html: &str, width: u32, dark: bool) -> Result<Capture, String>;
}

pub struct Native;

fn command() -> Result<std::process::Command, String> {
    let mut executable = std::env::current_exe().map_err(|e| e.to_string())?;
    if cfg!(test) {
        executable = executable
            .parent()
            .and_then(Path::parent)
            .ok_or("Missing build directory.")?
            .join(if cfg!(windows) { "shina.exe" } else { "shina" });
    }
    let mut command = std::process::Command::new(executable);
    command
        .arg("--html-native")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    // WebView2 environment overrides can replace our proxy arguments or profile.
    // Change only the helper's environment, never the user's configuration.
    for name in [
        "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
        "WEBVIEW2_USER_DATA_FOLDER",
        "WEBVIEW2_BROWSER_EXECUTABLE_FOLDER",
        "WEBVIEW2_RELEASE_CHANNEL_PREFERENCE",
        "WEBVIEW2_CHANNEL_SEARCH_KIND",
        "WEBVIEW2_RELEASE_CHANNELS",
        "WEBVIEW2_PIPE_FOR_SCRIPT_DEBUGGER",
        "WEBVIEW2_WAIT_FOR_SCRIPT_DEBUGGER",
        crate::mcp::TOKEN_ENV,
        crate::mcp::URL_ENV,
    ] {
        command.env_remove(name);
    }
    Ok(command)
}

impl Renderer for Native {
    fn capture(&self, html: &str, width: u32, dark: bool) -> Result<Capture, String> {
        if !cfg!(any(target_os = "macos", windows)) {
            return Err("The system HTML renderer is unavailable on this platform.".into());
        }
        let mut process =
            crate::process::Process::spawn(&mut command()?).map_err(|e| e.to_string())?;
        let request = json!({"html":html, "width":width, "dark":dark, "capture":true});
        let mut stdin = process.stdin.take().ok_or("Missing renderer input.")?;
        // A separate writer keeps a wedged native helper from blocking the caller forever.
        std::thread::spawn(move || {
            let _ = writeln!(stdin, "{request}");
        });
        let stdout = process.stdout.take().ok_or("Missing renderer output.")?;
        let (send, recv) = mpsc::channel();
        std::thread::spawn(move || {
            let mut line = String::new();
            let result = BufReader::new(stdout)
                .take(40 * 1024 * 1024)
                .read_line(&mut line)
                .map_err(|e| e.to_string())
                .and_then(|_| serde_json::from_str::<Value>(&line).map_err(|_| {
                    "The system HTML renderer could not start. Use html_render to publish without a snapshot.".to_owned()
                }));
            let _ = send.send(result);
        });
        let result = recv
            .recv_timeout(Duration::from_secs(7))
            .map_err(|_| "The HTML renderer exceeded its seven-second deadline.")??;
        if let Some(error) = result["error"].as_str() {
            return Err(error.into());
        }
        let png = crate::attachments::decode_base64(
            result["png"].as_str().ok_or("No PNG from renderer.")?,
        )
        .ok_or("Invalid PNG encoding.")?;
        let image =
            image::ImageReader::with_format(std::io::Cursor::new(&png), image::ImageFormat::Png)
                .into_dimensions()
                .map_err(|e| e.to_string())?;
        if png.len() > crate::html_render::MAX_PAGE
            || image.0 != width * 2
            || image.1 == 0
            || image.1 > crate::html_render::MAX_CAPTURE_HEIGHT * 2
        {
            return Err("The renderer returned invalid PNG dimensions.".into());
        }
        let mut console: Vec<ConsoleMessage> =
            serde_json::from_value(result["consoleMessages"].clone()).unwrap_or_default();
        console.truncate(20);
        for message in &mut console {
            message.text = message.text.chars().take(500).collect();
            if !matches!(message.level.as_str(), "log" | "info" | "warning" | "error") {
                message.level = "log".into();
            }
        }
        Ok(Capture {
            png,
            width: image.0,
            height: image.1,
            content_height: result["contentHeight"]
                .as_u64()
                .unwrap_or((image.1 / 2) as u64)
                .min(20000) as u32,
            console,
        })
    }
}

pub fn preview(
    input: &crate::html_render::Input,
    renderer: &dyn Renderer,
) -> Result<Value, String> {
    let capture = renderer.capture(&input.html, input.width, input.dark)?;
    Ok(json!({
        "width": input.width, "contentHeight":capture.content_height,
        "capturedHeight":capture.height / 2, "consoleMessages":capture.console,
        "missingImages":input.missing,
        "screenshot":{"mimeType":"image/png","data":crate::attachments::encode_base64(&capture.png),
            "width":input.width,"height":capture.height / 2}
    }))
}

struct Live {
    process: crate::process::Process,
    input: ChildStdin,
}

pub struct Windows {
    live: Mutex<HashMap<String, Live>>,
    theme: Mutex<Option<bool>>,
    fallback: crate::html_fallback::Fallback,
    opening: Mutex<HashSet<String>>,
    replies: Mutex<mpsc::Receiver<Result<Option<String>, String>>>,
    send: mpsc::Sender<Result<Option<String>, String>>,
}

impl Default for Windows {
    fn default() -> Self {
        let (send, replies) = mpsc::channel();
        Self {
            live: Default::default(),
            theme: Default::default(),
            fallback: Default::default(),
            opening: Default::default(),
            replies: Mutex::new(replies),
            send,
        }
    }
}

impl Windows {
    /// Attachment previews reuse the isolated native document host, not a public file URL.
    pub fn open_attachment_async(
        self: &Arc<Self>,
        attachment: crate::attachments::Attachment,
        dark: bool,
        wake: rune::Waker,
    ) {
        let windows = self.clone();
        std::thread::spawn(move || {
            let result = (|| {
                use std::io::Read;
                let path = crate::attachments::path_for(&attachment)
                    .ok_or("The attachment is unavailable.")?;
                let mut bytes = Vec::new();
                std::fs::File::open(path)
                    .map_err(|e| e.to_string())?
                    .take(crate::attachments::MAX_FILE_BYTES + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|e| e.to_string())?;
                if bytes.len() as u64 > crate::attachments::MAX_FILE_BYTES {
                    return Err("File exceeds 50 MiB.".into());
                }
                let mut input =
                    json!({"title":attachment.name,"width":960,"dark":dark,"capture":false});
                if attachment.mime == "application/pdf"
                    || attachment.name.to_ascii_lowercase().ends_with(".pdf")
                {
                    if !bytes.starts_with(b"%PDF-") {
                        return Err("Unable to load PDF.".into());
                    }
                    input["html"] = json!("");
                    input["pdfBase64"] = json!(crate::attachments::encode_base64(&bytes));
                } else if attachment.mime.starts_with("font/")
                    || attachment.name.rsplit('.').next().is_some_and(|e| {
                        matches!(
                            e.to_ascii_lowercase().as_str(),
                            "ttf" | "otf" | "woff" | "woff2"
                        )
                    })
                {
                    if bytes.len() > 10 * 1024 * 1024 {
                        return Err("Font previews are limited to 10 MB.".into());
                    }
                    input["attachmentPreview"] = json!(true);
                    input["html"] = json!(font_document(&bytes));
                } else if attachment.mime.starts_with("image/") {
                    let mime = crate::attachment_intake::native_mime(&attachment.name);
                    if !mime.starts_with("image/") {
                        return Err("Unable to load image.".into());
                    }
                    input["attachmentPreview"] = json!(true);
                    input["html"] = json!(format!(
                        "<!doctype html><style>html,body{{margin:0;width:100%;height:100%;display:grid;place-items:center}}img{{max-width:100%;max-height:100vh;object-fit:contain}}</style><img src=\"data:{mime};base64,{}\">",
                        crate::attachments::encode_base64(&bytes)
                    ));
                } else {
                    input["attachmentPreview"] = json!(true);
                    input["html"] = json!(String::from_utf8_lossy(&bytes));
                }
                windows.open_document(&format!("attachment-{}", attachment.id), &input)?;
                Ok(None)
            })();
            let _ = windows.send.send(result);
            wake();
        });
    }

    pub fn open_async(
        self: &Arc<Self>,
        directory: PathBuf,
        reference: crate::html_render::Reference,
        dark: bool,
        wake: rune::Waker,
    ) {
        if !self
            .opening
            .lock()
            .expect("HTML opens")
            .insert(reference.attachment_id.clone())
        {
            return;
        }
        let windows = self.clone();
        std::thread::spawn(move || {
            let outcome = windows.open(&directory, &reference, dark);
            windows
                .opening
                .lock()
                .expect("HTML opens")
                .remove(&reference.attachment_id);
            let _ = windows.send.send(outcome);
            wake();
        });
    }

    pub fn next(&self) -> Option<Result<Option<String>, String>> {
        self.replies.lock().expect("HTML replies").try_recv().ok()
    }

    /// Called by the card. No work here may run page script on the UI thread.
    fn open(
        &self,
        directory: &Path,
        reference: &crate::html_render::Reference,
        dark: bool,
    ) -> Result<Option<String>, String> {
        let html = crate::html_render::page(directory, reference)?;
        let input =
            json!({"html":html,"title":reference.title,"width":960,"dark":dark,"capture":false});
        match self.open_document(&reference.attachment_id, &input) {
            Ok(()) => Ok(None),
            Err(_) => self.fallback.url(directory, reference, dark).map(Some),
        }
    }

    fn open_document(&self, key: &str, request: &Value) -> Result<(), String> {
        let mut live = self.live.lock().expect("HTML windows");
        if let Some(window) = live.get_mut(key) {
            if window.process.try_wait().ok().flatten().is_none()
                && writeln!(window.input, "{}", json!({"focus":true})).is_ok()
            {
                return Ok(());
            }
            live.remove(key);
        }
        drop(live);
        let launch = (|| -> Result<Live, String> {
            if !cfg!(any(target_os = "macos", windows)) {
                return Err("No system webview.".into());
            }
            let mut process =
                crate::process::Process::spawn(&mut command()?).map_err(|e| e.to_string())?;
            let mut input = process.stdin.take().ok_or("Missing renderer input.")?;
            writeln!(input, "{}", request).map_err(|e| e.to_string())?;
            // The native helper acknowledges only after all security settings are installed.
            let stdout = process.stdout.take().ok_or("Missing renderer output.")?;
            let (send, recv) = mpsc::channel();
            std::thread::spawn(move || {
                let mut response = String::new();
                let _ = BufReader::new(stdout).read_line(&mut response);
                let _ = send.send(response);
            });
            let response = recv
                .recv_timeout(Duration::from_secs(3))
                .map_err(|_| "Native window creation timed out.")?;
            if serde_json::from_str::<Value>(&response)
                .ok()
                .is_none_or(|v| v["ready"] != true)
            {
                return Err("The system webview could not be created.".into());
            }
            Ok(Live { process, input })
        })();
        match launch {
            Ok(window) => {
                let mut window = window;
                if let Some(dark) = *self.theme.lock().expect("HTML theme") {
                    let _ = writeln!(window.input, "{}", json!({"dark":dark}));
                }
                self.live
                    .lock()
                    .expect("HTML windows")
                    .insert(key.to_owned(), window);
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    pub fn theme(&self, dark: bool) {
        let mut previous = self.theme.lock().expect("HTML theme");
        if *previous == Some(dark) {
            return;
        }
        *previous = Some(dark);
        self.live
            .lock()
            .expect("HTML windows")
            .retain(|_, window| writeln!(window.input, "{}", json!({"dark":dark})).is_ok());
    }
}

/// Only the trusted PDF host may embed a plugin. Published HTML retains object-src 'none'.
pub(crate) fn pdf_bytes(input: &Value) -> Result<Option<Vec<u8>>, String> {
    let Some(encoded) = input["pdfBase64"].as_str() else {
        return Ok(None);
    };
    if encoded.len() as u64 > crate::attachments::MAX_FILE_BYTES.div_ceil(3) * 4 {
        return Err("PDF exceeds 50 MiB.".into());
    }
    let bytes = crate::attachments::decode_base64(encoded).ok_or("Invalid PDF encoding.")?;
    if bytes.len() as u64 > crate::attachments::MAX_FILE_BYTES || !bytes.starts_with(b"%PDF-") {
        return Err("Unable to load PDF.".into());
    }
    Ok(Some(bytes))
}

/// KSG FontViewer's specimen, rendered by the existing platform FontFace engine.
fn font_document(bytes: &[u8]) -> String {
    let sizes = [12, 16, 24, 40, 64].into_iter().map(|size| format!(
        "<p style=\"font-size:{size}px\"><small>{size} px</small>The quick brown fox jumps over the lazy dog.</p>")).collect::<String>();
    let specimen = format!(
        "<!doctype html><meta charset=\"utf-8\"><style>@font-face{{font-family:preview;src:url('data:application/octet-stream;base64,{}')}}body{{margin:0;padding:32px;font-family:preview;overflow-wrap:anywhere}}h1{{font-size:40px;font-weight:normal}}p{{line-height:1.4}}small{{font:12px monospace;display:block;opacity:.6;margin-bottom:8px}}</style><h1>Font preview</h1><p style=\"font-size:24px\">ABCDEFGHIJKLMNOPQRSTUVWXYZ<br>abcdefghijklmnopqrstuvwxyz<br>0123456789 !?&amp;@#%(){{}}[]</p>{sizes}",
        crate::attachments::encode_base64(bytes)
    );
    format!(
        "{specimen}<script>{}</script>",
        include_str!("../assets/attachment-font-preview.js")
            .replace("__FONT_DATA__", &crate::attachments::encode_base64(bytes))
    )
}

#[cfg(windows)]
pub(crate) fn pdf_document(bytes: &[u8]) -> String {
    format!(
        "<!doctype html><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'; object-src data:; base-uri 'none'; form-action 'none'\"><style>html,body,embed{{margin:0;width:100%;height:100%;overflow:hidden}}</style><embed type=\"application/pdf\" src=\"data:application/pdf;base64,{}\">",
        crate::attachments::encode_base64(bytes)
    )
}

/// Early CLI entry. This mode never opens a Shina profile or starts MCP/client servers.
pub fn helper() -> Result<(), String> {
    #[cfg(any(target_os = "macos", windows))]
    {
        platform::run()
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        Err("The system HTML renderer is unavailable on this platform.".into())
    }
}

/// Bootstrap for the isolated document, ported from packages/shared/src/htmlRender.ts.
/// CSP is in the trusted outer document, not attacker-controlled markup.
pub fn document(html: &str, dark: bool) -> String {
    #[cfg(target_os = "macos")]
    let rewritten = crate::html_network::rewrite(html);
    #[cfg(target_os = "macos")]
    let html = rewritten.as_str();
    let theme = crate::ui::html_theme(dark);
    let css: String = theme["variables"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| format!("{k}:{};", v.as_str().unwrap()))
        .collect();
    let base = format!(
        r#"<meta charset="utf-8"><style id="shina-theme">:root{{color-scheme:{};{css}}}html{{box-sizing:border-box;padding:8px;color:var(--foreground);font:14px/1.5 var(--font-sans);background:var(--background)}}body{{margin:0}}code,kbd,pre,samp{{font-family:var(--font-mono)}}</style>"#,
        if dark { "dark" } else { "light" }
    );
    let script = include_str!("../assets/html/page.js");
    // srcdoc HTML escaping prevents </iframe>, quote and entity injection into the host.
    let source = format!("{base}<script>{script}</script>{html}")
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!(
        r#"<!doctype html><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline' 'unsafe-eval' data: blob: https: http: shina-http: shina-https:; style-src 'unsafe-inline' data: https: http: shina-http: shina-https:; img-src data: blob: https: http: shina-http: shina-https:; font-src data: https: http: shina-http: shina-https:; media-src data: blob: https: http: shina-http: shina-https:; frame-src 'self' about:; connect-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'"><style>html,body{{margin:0;width:100%;height:100%;overflow:hidden}}iframe{{display:block;border:0;width:100%;height:100%}}</style><iframe sandbox="allow-scripts allow-forms" allow="camera 'none'; microphone 'none'; geolocation 'none'; clipboard-read 'none'; clipboard-write 'none'; display-capture 'none'; usb 'none'; serial 'none'; bluetooth 'none'" srcdoc="{source}"></iframe><script>window.shinaInitialTheme={theme};window.shinaOpenLink=url=>{{{link}}};{host}</script>"#,
        host = include_str!("../assets/html/host.js"),
        link = if cfg!(target_os = "macos") {
            "window.webkit.messageHandlers.link.postMessage(url);"
        } else {
            "window.chrome.webview.postMessage({link:url});"
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_pdf_inputs_are_bounded_and_cannot_turn_published_html_into_a_plugin_host() {
        assert!(pdf_bytes(&json!({"html":"ordinary"})).unwrap().is_none());
        assert!(pdf_bytes(&json!({"pdfBase64":"not base64!"})).is_err());
        assert!(
            pdf_bytes(&json!({"pdfBase64":crate::attachments::encode_base64(b"not PDF")})).is_err()
        );
        assert_eq!(
            pdf_bytes(&json!({"pdfBase64":crate::attachments::encode_base64(b"%PDF-1.4\n")}))
                .unwrap()
                .unwrap(),
            b"%PDF-1.4\n"
        );
        assert!(
            document("<embed src='data:application/pdf;base64,AA'>", false)
                .contains("object-src 'none'")
        );
        let specimen = font_document(b"fixture");
        for size in [12, 16, 24, 40, 64] {
            assert!(specimen.contains(&format!("{size} px")));
        }
    }
    #[test]
    fn hostile_markup_cannot_escape_the_opaque_frame() {
        let page = document(
            r#"</iframe><script>window.hostile=true</script><img src="file:///private">"#,
            false,
        );
        assert_eq!(page.matches("<iframe ").count(), 1);
        assert_eq!(page.matches("</iframe>").count(), 1);
        assert!(page.contains("&lt;/iframe&gt;"));
        assert!(page.contains("sandbox=\"allow-scripts allow-forms\""));
        assert!(!page.contains("allow-same-origin"));
        assert!(page.contains("connect-src 'none'"));
        assert!(page.contains("clipboard-read 'none'"));
    }

    #[test]
    fn helper_does_not_inherit_mcp_tokens_or_webview_configuration_overrides() {
        let command = command().unwrap();
        for name in [
            crate::mcp::TOKEN_ENV,
            crate::mcp::URL_ENV,
            "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
            "WEBVIEW2_USER_DATA_FOLDER",
        ] {
            assert!(
                command
                    .get_envs()
                    .any(|(key, value)| key == name && value.is_none())
            );
        }
    }
}

use std::io::Read;
