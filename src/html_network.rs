//! Fail-closed public resource loader and Windows CONNECT proxy.
//! Resolve once, reject every private answer, then connect to a checked numeric address.
use std::{
    io::Read,
    net::{SocketAddr, TcpListener, ToSocketAddrs},
    time::Duration,
};
#[cfg(any(windows, test))]
use std::{
    io::Write,
    net::{Shutdown, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Instant,
};

#[cfg(any(windows, test))]
pub fn proxy() -> Result<u16, String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let count = Arc::new(AtomicUsize::new(0));
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            if count.fetch_add(1, Ordering::AcqRel) >= 32 {
                count.fetch_sub(1, Ordering::AcqRel);
                continue;
            }
            let count = count.clone();
            std::thread::spawn(move || {
                let _ = connect(stream);
                count.fetch_sub(1, Ordering::AcqRel);
            });
        }
    });
    Ok(port)
}

fn addresses(url: &url::Url) -> Result<Vec<SocketAddr>, String> {
    if !crate::html_render::public_url(url.as_str()) {
        return Err("Private resource URL refused.".into());
    }
    let host = match url.host().ok_or("Missing resource host.")? {
        url::Host::Domain(host) => host.to_owned(),
        url::Host::Ipv4(ip) => ip.to_string(),
        url::Host::Ipv6(ip) => ip.to_string(),
    };
    let addresses: Vec<_> = (
        host.as_str(),
        url.port_or_known_default()
            .ok_or("Missing resource port.")?,
    )
        .to_socket_addrs()
        .map_err(|e| e.to_string())?
        .collect();
    if addresses.is_empty()
        || addresses.iter().any(|addr| {
            !crate::html_render::public_ip(addr.ip())
                || TcpListener::bind(SocketAddr::new(addr.ip(), 0)).is_ok()
        })
    {
        return Err("Private resource address refused.".into());
    }
    Ok(addresses)
}

#[cfg(target_os = "macos")]
pub struct Resource {
    pub bytes: Vec<u8>,
    pub mime: String,
}

/// WebKit bypasses proxies for loopback. Its custom scheme uses this pinned loader;
/// a content rule blocks every direct HTTP request in the webview.
#[cfg(target_os = "macos")]
pub fn resource(raw: &str) -> Result<Resource, String> {
    let raw = raw
        .replacen("shina-https:", "https:", 1)
        .replacen("shina-http:", "http:", 1);
    let mut url = url::Url::parse(&raw).map_err(|e| e.to_string())?;
    for _ in 0..6 {
        let addresses = addresses(&url)?;
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(2))
            .resolve_to_addrs(url.host_str().ok_or("Missing resource host.")?, &addresses)
            .build()
            .map_err(|e| e.to_string())?;
        let mut response = client.get(url.clone()).send().map_err(|e| e.to_string())?;
        if response.status().is_redirection() {
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or("Invalid resource redirect.")?;
            url = url.join(location).map_err(|e| e.to_string())?;
            continue;
        }
        if !response.status().is_success() {
            return Err("Resource unavailable.".into());
        }
        let mime = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .split(';')
            .next()
            .unwrap_or("")
            .to_owned();
        let mut bytes = Vec::new();
        response
            .by_ref()
            .take(crate::html_render::MAX_PAGE as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > crate::html_render::MAX_PAGE {
            return Err("Remote resource exceeds 25 MiB.".into());
        }
        if (mime.starts_with("text/")
            || mime.contains("javascript")
            || mime.contains("json")
            || mime == "image/svg+xml")
            && let Ok(text) = std::str::from_utf8(&bytes)
        {
            bytes = rewrite(text).into_bytes();
        }
        return Ok(Resource { bytes, mime });
    }
    Err("Too many resource redirects.".into())
}

#[cfg(target_os = "macos")]
pub fn rewrite(html: &str) -> String {
    // Rewrite resource locations, not arbitrary strings such as SVG namespace URIs.
    regex::Regex::new(r#"(?i)((?:\b(?:src|href|poster)\s*=\s*["']?\s*)|(?:url\(\s*["']?\s*)|(?:@import\s*["']\s*))(https?)://"#).unwrap()
        .replace_all(html,|captures:&regex::Captures|
            format!("{}shina-{}://",&captures[1],captures[2].to_ascii_lowercase()))
        .into_owned()
}

#[cfg(any(windows, test))]
fn connect(mut client: TcpStream) -> std::io::Result<()> {
    client.set_read_timeout(Some(Duration::from_secs(3)))?;
    client.set_write_timeout(Some(Duration::from_secs(3)))?;
    let mut request = Vec::new();
    let mut byte = [0];
    let deadline = Instant::now() + Duration::from_secs(3);
    while request.len() < 8192 {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(std::io::ErrorKind::TimedOut.into());
        }
        client.set_read_timeout(Some(remaining))?;
        client.read_exact(&mut byte)?;
        request.push(byte[0]);
        if request.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let request = String::from_utf8_lossy(&request);
    let parts: Vec<_> = request
        .lines()
        .next()
        .unwrap_or("")
        .split_whitespace()
        .collect();
    if parts.len() != 3
        || parts[0] != "CONNECT"
        || !crate::html_render::public_url(&format!("https://{}/", parts[1]))
    {
        return client.write_all(
            b"HTTP/1.1 403 Forbidden\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
        );
    }
    let url = url::Url::parse(&format!("https://{}/", parts[1])).map_err(std::io::Error::other)?;
    let Ok(addresses) = addresses(&url) else {
        return client.write_all(
            b"HTTP/1.1 403 Forbidden\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
        );
    };
    let mut remote = addresses
        .iter()
        .find_map(|address| TcpStream::connect_timeout(address, Duration::from_secs(3)).ok())
        .ok_or_else(|| std::io::Error::other("Public host unavailable"))?;
    client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")?;
    client.set_read_timeout(Some(Duration::from_secs(30)))?;
    remote.set_read_timeout(Some(Duration::from_secs(30)))?;
    remote.set_write_timeout(Some(Duration::from_secs(3)))?;
    let mut remote_write = remote.try_clone()?;
    let mut client_read = client.try_clone()?;
    std::thread::spawn(move || {
        let _ = std::io::copy(&mut client_read, &mut remote_write);
        let _ = remote_write.shutdown(Shutdown::Both);
    });
    let _ = std::io::copy(&mut remote, &mut client);
    let _ = client.shutdown(Shutdown::Both);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn connect_proxy_never_reaches_private_addresses() {
        let canary = TcpListener::bind("127.0.0.1:0").unwrap();
        canary.set_nonblocking(true).unwrap();
        let port = proxy().unwrap();
        for host in [
            "127.0.0.1",
            "2130706433",
            "0x7f000001",
            "[::1]",
            "[::ffff:127.0.0.1]",
        ] {
            let mut socket = TcpStream::connect(("127.0.0.1", port)).unwrap();
            write!(
                socket,
                "CONNECT {host}:{} HTTP/1.1\r\n\r\n",
                canary.local_addr().unwrap().port()
            )
            .unwrap();
            let mut response = String::new();
            socket.read_to_string(&mut response).unwrap();
            assert!(response.starts_with("HTTP/1.1 403"), "{host}: {response}");
        }
        assert!(canary.accept().is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn custom_resource_loader_checks_urls_before_network_access() {
        for url in [
            "shina-http://127.0.0.1/private",
            "shina-https://[::ffff:127.0.0.1]/",
            "shina-http://0x7f000001/",
            "file:///etc/passwd",
        ] {
            assert!(resource(url).is_err(), "{url}");
        }
        assert_eq!(
            rewrite("<img src='HTTPS://example.com/image.png'>"),
            "<img src='shina-https://example.com/image.png'>"
        );
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><script>document.createElementNS("http://www.w3.org/2000/svg","rect")</script></svg>"#;
        assert_eq!(rewrite(svg), svg);
    }
}
