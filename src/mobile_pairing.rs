//! Phone enrollment over the tailnet. `shina pair --mobile <tailnet-host>` prints a
//! single-use `shina://pair/v1` link and a terminal QR. The link carries the validated
//! tailnet destination, the expected server identity and the code's expiry; the
//! one-time code travels in the fragment so it never appears in an HTTP path or query.
//!
//! Envelope v1:
//!
//! ```text
//! shina://pair/v1?host=<host>&port=<port>&serverId=<uuid>&expiresAt=<ms>#code=<code>
//! ```
//!
//! - `host`: MagicDNS name, or a Tailscale IPv4/IPv6 literal without brackets.
//! - `port`: canonical decimal, nonzero. The tailnet listener's port, which is the
//!   `--port` of `shina serve` or `peer::PORT`, never `Discovery.port` (loopback).
//! - `serverId`: canonical hyphenated UUID. The redeemer checks it against
//!   `GET /v1/identity` and sends it back as `expectedServerId` in `POST /v1/pair`.
//! - `expiresAt`: Unix milliseconds, canonical decimal, nonzero.
//! - `code`: exactly `[0-9A-F]{12}`, the only credential in the link.
//!
//! The parser accepts only this shape. It rejects userinfo and other authorities,
//! other schemes and versions, unknown, duplicate or empty fields, a code anywhere
//! but the fragment, public, LAN, loopback and wildcard IP literals, malformed ids,
//! ports, expiries and codes, and oversized input. Percent-encoding is not part of
//! v1: no field ever contains `%`.
//!
//! Redemption, on the phone: `PairLink::parse`, `resolve` the host and dial only the
//! returned numeric address, verify `serverId` from `GET /v1/identity`, then POST
//! `{code, deviceId, expectedServerId}` to `/v1/pair`. The code is never transmitted
//! to an address that has not passed every check.
use serde_json::{Value, json};
use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use uuid::Uuid;

const PREFIX: &str = "shina://pair/v1?";
const CODE_LEN: usize = 12;
#[cfg_attr(not(test), allow(dead_code))]
const MAX_LINK_LEN: usize = 512;

/// One parsed or freshly minted enrollment link. `Debug` never shows `code`.
#[derive(Clone, PartialEq, Eq)]
pub struct PairLink {
    host: String,
    port: u16,
    server_id: Uuid,
    expires_at: u64,
    code: String,
}

impl PairLink {
    /// A link whose fields have already passed the parser's rules. `code` is the
    /// credential; it leaves this process only through `encode` and redemption.
    pub fn new(
        host: &str,
        port: u16,
        server_id: Uuid,
        expires_at: u64,
        code: &str,
    ) -> Result<Self, String> {
        if port == 0 {
            return Err("Invalid port.".into());
        }
        if expires_at == 0 {
            return Err("Invalid expiry.".into());
        }
        Ok(Self {
            host: host_field(host)?,
            port,
            server_id,
            expires_at,
            code: code_field(code)?,
        })
    }

    /// The strict v1 parser. Field order in the text does not matter; the formatter's
    /// order is canonical.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        if text.len() > MAX_LINK_LEN
            || !text.is_ascii()
            || text
                .bytes()
                .any(|b| !(0x21..0x7f).contains(&b) || b == b'%')
        {
            return Err("Not a Shina mobile pairing link.".into());
        }
        let rest = text
            .strip_prefix(PREFIX)
            .ok_or("Not a Shina mobile pairing link.")?;
        let (query, fragment) = rest
            .split_once('#')
            .ok_or("Missing pairing code fragment.")?;
        let mut host = None;
        let mut port = None;
        let mut server_id = None;
        let mut expires_at = None;
        for field in query.split('&') {
            let (key, value) = field.split_once('=').ok_or("Invalid link field.")?;
            if value.is_empty() {
                return Err("Invalid link field.".into());
            }
            let slot = match key {
                "host" => &mut host,
                "port" => &mut port,
                "serverId" => &mut server_id,
                "expiresAt" => &mut expires_at,
                "code" => return Err("The pairing code belongs in the link fragment.".into()),
                _ => return Err("Unknown link field.".into()),
            };
            if slot.is_some() {
                return Err("Duplicate link field.".into());
            }
            *slot = Some(value);
        }
        let code = fragment
            .strip_prefix("code=")
            .ok_or("Missing pairing code fragment.")?;
        Ok(Self {
            host: host_field(host.ok_or("Missing pairing host.")?)?,
            port: port_field(port.ok_or("Missing pairing port.")?)?,
            server_id: server_id_field(server_id.ok_or("Missing server identity.")?)?,
            expires_at: expiry_field(expires_at.ok_or("Missing expiry.")?)?,
            code: code_field(code)?,
        })
    }

    /// The canonical link text. Intentional output only; this contains the code.
    pub fn encode(&self) -> String {
        format!(
            "{PREFIX}host={}&port={}&serverId={}&expiresAt={}#code={}",
            self.host, self.port, self.server_id, self.expires_at, self.code
        )
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn host(&self) -> &str {
        &self.host
    }
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn port(&self) -> u16 {
        self.port
    }
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn server_id(&self) -> Uuid {
        self.server_id
    }
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn expires_at(&self) -> u64 {
        self.expires_at
    }
    /// The one-time credential, for the redemption POST and nothing else.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn code(&self) -> &str {
        &self.code
    }
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn expired(&self, now: u64) -> bool {
        now >= self.expires_at
    }

    /// The only address a redeemer may dial. `peer::resolve` permits loopback for
    /// desktop peers; a phone cannot use it, so the pinned address must be tailnet.
    /// After connecting, check `GET /v1/identity`'s `serverId` and send
    /// `expectedServerId` with the code.
    #[allow(dead_code)]
    pub fn resolve(&self) -> Result<SocketAddr, String> {
        resolve_destination(&self.host, self.port)
    }

    /// UTF-8 block QR for a terminal, dark modules as light glyphs so it scans on
    /// the dark backgrounds terminals default to.
    pub fn terminal_qr(&self) -> Result<String, String> {
        let qr = qrcode::QrCode::new(self.encode().as_bytes())
            .map_err(|_| "Could not encode the pairing QR.")?;
        Ok(qr
            .render::<qrcode::render::unicode::Dense1x2>()
            .dark_color(qrcode::render::unicode::Dense1x2::Light)
            .light_color(qrcode::render::unicode::Dense1x2::Dark)
            .build())
    }

    /// The explicit local CLI output: link text, expiry and tailnet prerequisite,
    /// then the QR. This is the one place the code is deliberately shown.
    pub fn print(&self) -> Result<(), String> {
        let qr = self.terminal_qr()?;
        println!(
            "Mobile pairing link: {}\nExpires in 5 minutes. One use. The phone needs Tailscale on this tailnet.\n\n{qr}",
            self.encode()
        );
        Ok(())
    }
}

impl fmt::Debug for PairLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PairLink")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("server_id", &self.server_id)
            .field("expires_at", &self.expires_at)
            .field("code", &"<redacted>")
            .finish()
    }
}

/// `shina pair --mobile <tailnet-host>`: refuse before minting unless the server
/// really answers on the tailnet. No credential is created or transmitted until the
/// explicit host has resolved to a tailnet address and `/v1/identity` has answered
/// with this server's id.
pub async fn issue(profile: &Path, host: &str) -> Result<PairLink, String> {
    let host = normalize_host(host)?;
    let discovery = crate::server_auth::Discovery::read(profile)
        .map_err(|_| "No running Shina server. Start one with `shina serve --tailnet`.")?;
    let server_id =
        Uuid::parse_str(&discovery.server_id).map_err(|_| "Invalid server identity.")?;
    let port = tailnet_port(profile)?;
    let address = resolve_destination(&host, port)?;
    let identity = crate::peer::describe(address).await?;
    if identity["serverId"].as_str() != Some(discovery.server_id.as_str()) {
        return Err(
            "The tailnet endpoint belongs to a different server. No pairing code was created."
                .into(),
        );
    }
    let value = crate::client::local_rpc(profile, "devices.pair", json!({"peer": false})).await?;
    PairLink::new(
        &host,
        port,
        server_id,
        value["expiresAt"]
            .as_u64()
            .ok_or("Missing pairing expiry.")?,
        value["code"].as_str().ok_or("Missing pairing code.")?,
    )
}

/// The port a phone dials, from the listener record `serve` wrote. `Discovery.port`
/// is the loopback port and is random unless `--port` was passed, so it is never a
/// mobile destination. A missing record falls back to the default; the identity
/// probe in `issue` still verifies reachability.
fn tailnet_port(profile: &Path) -> Result<u16, String> {
    let Ok(bytes) = std::fs::read(profile.join("server-listen.json")) else {
        return Ok(crate::peer::PORT);
    };
    let listen: Value = serde_json::from_slice(&bytes).map_err(|_| "Invalid listener record.")?;
    if listen["tailnet"] != true {
        return Err(
            "The running server is not listening on the tailnet. Restart it with `shina serve --tailnet` before pairing a phone."
                .into(),
        );
    }
    match listen["port"].as_u64() {
        Some(port) => u16::try_from(port)
            .ok()
            .filter(|port| *port != 0)
            .ok_or("Invalid listener port.".into()),
        None => Ok(crate::peer::PORT),
    }
}

/// The CLI's host argument: a bare MagicDNS name or tailnet IP. Brackets around an
/// IPv6 literal are accepted here only; the stored form never has them. The port is
/// never taken from this argument.
fn normalize_host(raw: &str) -> Result<String, String> {
    let host = raw.trim();
    let host = host
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host);
    host_field(host)
}

/// The shared host rule. Literals must already be tailnet addresses; MagicDNS names
/// pass on syntax and are pinned at redemption by `resolve_destination`.
fn host_field(raw: &str) -> Result<String, String> {
    if let Ok(ip) = raw.parse::<IpAddr>() {
        if crate::peer::tailnet_address(ip) {
            return Ok(ip.to_string());
        }
        return Err(
            "The host must be a Tailscale address, not a public, LAN, loopback or wildcard one."
                .into(),
        );
    }
    if dns_name(raw) {
        return Ok(raw.to_owned());
    }
    Err("Enter a MagicDNS host or a Tailscale IP.".into())
}

fn dns_name(host: &str) -> bool {
    host.len() <= 253
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                && !label.starts_with('-')
                && !label.ends_with('-')
        })
}

#[cfg_attr(not(test), allow(dead_code))]
fn port_field(raw: &str) -> Result<u16, String> {
    let port: u16 = raw.parse().map_err(|_| "Invalid port.")?;
    if port == 0 || raw != port.to_string() {
        return Err("Invalid port.".into());
    }
    Ok(port)
}

#[cfg_attr(not(test), allow(dead_code))]
fn server_id_field(raw: &str) -> Result<Uuid, String> {
    let id = Uuid::parse_str(raw).map_err(|_| "Invalid server identity.")?;
    if raw != id.to_string() {
        return Err("Invalid server identity.".into());
    }
    Ok(id)
}

#[cfg_attr(not(test), allow(dead_code))]
fn expiry_field(raw: &str) -> Result<u64, String> {
    let expiry: u64 = raw.parse().map_err(|_| "Invalid expiry.")?;
    if expiry == 0 || raw != expiry.to_string() {
        return Err("Invalid expiry.".into());
    }
    Ok(expiry)
}

fn code_field(raw: &str) -> Result<String, String> {
    if raw.len() == CODE_LEN
        && raw
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'A'..=b'F').contains(&b))
    {
        Ok(raw.to_owned())
    } else {
        Err("Invalid pairing code.".into())
    }
}

/// Resolve `host` and keep only a tailnet dial address. `peer::resolve` already
/// refuses public and LAN answers; a mobile link additionally refuses loopback.
fn resolve_destination(host: &str, port: u16) -> Result<SocketAddr, String> {
    let authority = match host.parse::<IpAddr>() {
        Ok(ip) => SocketAddr::new(ip, port).to_string(),
        Err(_) => format!("{host}:{port}"),
    };
    let address = crate::peer::resolve(&authority)?;
    if !crate::peer::tailnet_address(address.ip()) {
        return Err("The host must resolve to a Tailscale address.".into());
    }
    Ok(address)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "0e2f5f36-0d3c-4b4a-9c7a-1f0e2d3c4b5a";
    const CODE: &str = "012345ABCDEF";

    fn link() -> PairLink {
        PairLink::new(
            "100.90.1.2",
            47329,
            ID.parse().unwrap(),
            1_760_000_000_000,
            CODE,
        )
        .unwrap()
    }

    fn link_for(host: &str) -> String {
        format!("{PREFIX}host={host}&port=47329&serverId={ID}&expiresAt=1760000000000#code={CODE}")
    }

    #[test]
    fn encodes_the_envelope_and_round_trips() {
        let link = link();
        let text = link.encode();
        assert_eq!(text, link_for("100.90.1.2"));
        let (query, fragment) = text.split_once('#').unwrap();
        assert!(!query.contains("code"), "{query}");
        assert_eq!(fragment, format!("code={CODE}"));
        assert_eq!(PairLink::parse(&text).unwrap(), link);
        for host in [
            "100.64.0.1",
            "100.127.255.254",
            "fd7a:115c:a1e0::1",
            "fd7a:115c:a1e0:0000:0000:0000:0000:0001",
            "studio.tail-abc.ts.net",
            "studio",
            "a-b.0-9.ts.net",
        ] {
            assert_eq!(
                PairLink::parse(&link_for(host)).unwrap().host(),
                PairLink::new(host, 47329, ID.parse().unwrap(), 1, CODE)
                    .unwrap()
                    .host(),
                "{host}"
            );
        }
    }

    #[test]
    fn rejects_nontailnet_literal_destinations() {
        for host in [
            "127.0.0.1",
            "::1",
            "0.0.0.0",
            "::",
            "192.168.1.4",
            "10.0.0.9",
            "172.16.4.4",
            "169.254.1.1",
            "8.8.8.8",
            "100.63.255.255",
            "100.128.0.1",
            "fd00::1",
            "fe80::1",
            "224.0.0.1",
        ] {
            assert!(PairLink::parse(&link_for(host)).is_err(), "{host}");
            assert!(
                PairLink::new(host, 47329, ID.parse().unwrap(), 1, CODE).is_err(),
                "{host}"
            );
        }
    }

    #[test]
    fn rejects_malformed_codes() {
        let ok = link_for("100.90.1.2");
        for code in [
            "",
            "012345abcdef",
            "012345abcdeff",
            "012345ABCDEF0",
            "012345ABCDE",
            "012345ABCDEG",
            "012345ABCDE\n",
            "012345ABCDE=",
            "012345ABCDE/",
        ] {
            assert!(PairLink::parse(&ok.replace(CODE, code)).is_err(), "{code}");
        }
        assert!(PairLink::parse(&ok.replace("#code=", "#")).is_err());
        assert!(PairLink::parse(&ok.replace("#code=", "#xcode=")).is_err());
    }

    #[test]
    fn debug_redacts_the_code_and_expiry_is_checked() {
        let link = link();
        let debug = format!("{link:?}");
        assert!(!debug.contains(CODE), "{debug}");
        assert!(debug.contains("<redacted>") && debug.contains("100.90.1.2"));
        assert!(!link.expired(1_760_000_000_000 - 1));
        assert!(link.expired(1_760_000_000_000));
        assert_eq!(link.port(), 47329);
        assert_eq!(link.server_id(), Uuid::parse_str(ID).unwrap());
        assert_eq!(link.expires_at(), 1_760_000_000_000);
        assert_eq!(link.code(), CODE);
    }
}
