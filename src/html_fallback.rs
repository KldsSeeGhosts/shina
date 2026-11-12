//! A dedicated, capability-addressed listener. It cannot route to MCP or the client API.
use std::{
    collections::HashMap,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

// A browser fallback has no request interceptor, so deny all network loads, not just fetch.
// `sandbox` gives a top-level browser document an opaque origin.
pub const CSP: &str = "sandbox allow-scripts allow-forms; default-src 'none'; script-src 'unsafe-inline' 'unsafe-eval' data: blob:; style-src 'unsafe-inline' data:; img-src data: blob:; font-src data:; media-src data: blob:; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'";

type Pages = Arc<Mutex<HashMap<String, String>>>;

#[derive(Default)]
pub struct Fallback {
    server: Mutex<Option<(u16, Pages)>>,
    urls: Mutex<HashMap<String, String>>,
}

impl Fallback {
    pub fn url(
        &self,
        directory: &Path,
        reference: &crate::html_render::Reference,
        dark: bool,
    ) -> Result<String, String> {
        let html = crate::html_render::page(directory, reference)?;
        let key = format!("{}-{dark}", reference.attachment_id);
        if let Some(url) = self.urls.lock().expect("HTML URLs").get(&key).cloned() {
            return Ok(url);
        }
        let mut server = self.server.lock().expect("HTML fallback");
        if server.is_none() {
            let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
            let port = listener.local_addr().map_err(|e| e.to_string())?.port();
            let pages: Pages = Default::default();
            let shared = pages.clone();
            std::thread::spawn(move || {
                for stream in listener.incoming().flatten() {
                    // Sequential, bounded request reads. No unbounded connection thread spawn.
                    let _ = serve(stream, port, &shared);
                }
            });
            *server = Some((port, pages));
        }
        let (port, pages) = server.as_ref().unwrap();
        let token = uuid::Uuid::new_v4().simple().to_string();
        let path = format!("/{token}/page.html");
        let variables = crate::ui::html_theme(dark);
        let css: String = variables["variables"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| format!("{k}:{};", v.as_str().unwrap()))
            .collect();
        pages.lock().expect("HTML pages").insert(path.clone(), format!("<!doctype html><meta charset=\"utf-8\"><style>:root{{{css}}}body{{background:var(--background);color:var(--foreground);font-family:var(--font-sans)}}</style>{html}"));
        let url = format!("http://127.0.0.1:{port}{path}");
        self.urls
            .lock()
            .expect("HTML URLs")
            .insert(key, url.clone());
        Ok(url)
    }
}

fn serve(mut stream: TcpStream, port: u16, pages: &Pages) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let mut request = Vec::new();
    let mut byte = [0];
    let deadline = Instant::now() + Duration::from_secs(2);
    while request.len() < 8192 {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(std::io::ErrorKind::TimedOut.into());
        }
        stream.set_read_timeout(Some(remaining))?;
        if stream.read(&mut byte)? == 0 {
            return Ok(());
        }
        request.push(byte[0]);
        if request.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let text = String::from_utf8_lossy(&request);
    let mut lines = text.split("\r\n");
    let first = lines.next().unwrap_or("");
    let parts: Vec<_> = first.split_whitespace().collect();
    let mut host_ok = false;
    let mut origin = false;
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("host") {
                host_ok = value.trim() == format!("127.0.0.1:{port}");
            }
            if name.eq_ignore_ascii_case("origin") {
                origin = true;
            }
        }
    }
    let html =
        if host_ok && !origin && parts.len() == 3 && parts[0] == "GET" && parts[2] == "HTTP/1.1" {
            pages.lock().expect("HTML pages").get(parts[1]).cloned()
        } else {
            None
        };
    let (status, body) = html
        .as_deref()
        .map_or(("404 Not Found", "Not found"), |html| ("200 OK", html));
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nContent-Security-Policy: {CSP}\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nPermissions-Policy: camera=(), microphone=(), geolocation=(), clipboard-read=(), clipboard-write=(), usb=()\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn get(url: &str, route: Option<&str>, headers: &str) -> String {
        let parsed = url::Url::parse(url).unwrap();
        let address = format!("127.0.0.1:{}", parsed.port().unwrap());
        let mut stream = TcpStream::connect(&address).unwrap();
        write!(
            stream,
            "GET {} HTTP/1.1\r\nHost: {address}\r\n{headers}\r\n",
            route.unwrap_or(parsed.path())
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    }

    #[test]
    fn only_published_capability_urls_are_served_with_sandbox_headers() {
        let reference = crate::html_render::tests::fixture(false);
        let directory = crate::store::directory().unwrap().join("attachments");
        let fallback = Fallback::default();
        let url = fallback.url(&directory, &reference, false).unwrap();
        let response = get(&url, None, "");
        assert!(response.starts_with("HTTP/1.1 200"));
        for header in [
            "Content-Security-Policy: sandbox allow-scripts allow-forms;",
            "connect-src 'none'",
            "X-Content-Type-Options: nosniff",
            "Cache-Control: no-store",
        ] {
            assert!(response.contains(header), "{response}");
        }
        assert!(!response.to_ascii_lowercase().contains("set-cookie"));
        assert!(get(&url, Some("/mcp"), "").starts_with("HTTP/1.1 404"));
        assert!(get(&url, Some("/v1/blobs/page.html"), "").starts_with("HTTP/1.1 404"));
        assert!(get(&url, Some("/../page.html"), "").starts_with("HTTP/1.1 404"));
        assert!(get(&url, None, "Origin: null\r\n").starts_with("HTTP/1.1 404"));
        let mut unbound = reference;
        unbound.thread_id = 2;
        assert!(fallback.url(&directory, &unbound, false).is_err());
    }
}
