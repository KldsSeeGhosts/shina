//! Direct, user-paired clients. No provider credentials or host paths cross installations.
//! Address discovery follows KSG packages/tailscale; redirects never receive credentials.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    net::{IpAddr, SocketAddr, ToSocketAddrs},
    path::{Path, PathBuf},
    time::Duration,
};

pub const PORT: u16 = 47329;

/// Outbound server link metadata. This type never contains a credential.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Peer {
    pub server_id: String,
    pub host: String,
    pub label: String,
    pub linked_at: u64,
    pub capabilities: Value,
    pub last_reached_at: Option<u64>,
    pub last_error: Option<String>,
}
impl Peer {
    pub fn summary(&self) -> Value {
        let mut value = serde_json::to_value(self).expect("peer metadata");
        value["reachable"] = json!(
            self.last_error.is_none()
                && self
                    .last_reached_at
                    .is_some_and(|at| crate::lifecycle::now().saturating_sub(at) < 90_000)
        );
        value
    }
    pub fn credential(&self, profile: &Path) -> Result<Link, String> {
        let path = profile
            .join("peers")
            .join(&self.server_id)
            .join("credential.json");
        let link: Link = serde_json::from_slice(
            &std::fs::read(path).map_err(|_| "Peer credential unavailable.")?,
        )
        .map_err(|_| "Invalid peer credential.")?;
        if link.server_id != self.server_id || link.host != self.host {
            return Err("Peer credential identity mismatch.".into());
        }
        Ok(link)
    }
}

async fn revoke_peer(link: &Link) -> Result<Value, String> {
    crate::client::remote_command(
        link,
        crate::protocol::Command::new("devices.revoke", json!({"deviceId":link.device_id})),
    )
    .await
}

pub async fn describe(address: SocketAddr) -> Result<Value, String> {
    let response = http()?
        .get(format!("http://{address}/v1/identity"))
        .send()
        .await
        .map_err(|_| "Machine identity endpoint is unavailable.")?;
    if !response.status().is_success() {
        return Err("Machine identity endpoint refused.".into());
    }
    let value: Value = response
        .json()
        .await
        .map_err(|_| "Invalid machine identity.")?;
    uuid::Uuid::parse_str(
        value["serverId"]
            .as_str()
            .ok_or("Missing server identity.")?,
    )
    .map_err(|_| "Invalid server identity.")?;
    Ok(value)
}

/// The numeric address is pinned for both the public identity probe and authenticated call.
pub async fn pin(link: &Link) -> Result<Link, String> {
    let address = link.address()?;
    if describe(address).await?["serverId"] != link.server_id {
        return Err("Machine identity changed. No credential was sent.".into());
    }
    let mut pinned = link.clone();
    pinned.host = address.to_string();
    Ok(pinned)
}

pub async fn link_peer(
    profile: &Path,
    database: &crate::database::Database,
    host: &str,
    code: &str,
) -> Result<Peer, String> {
    let address = resolve(host)?;
    let identity = describe(address).await?;
    if std::fs::read_to_string(profile.join("server-id"))
        .ok()
        .as_deref()
        == identity["serverId"].as_str()
    {
        return Err("A server cannot link to itself.".into());
    }
    if identity["peerDelegation"] != true {
        return Err("Update the target to support peer delegation.".into());
    }
    let server_id = identity["serverId"]
        .as_str()
        .expect("validated identity")
        .to_owned();
    let previous = database
        .peers()?
        .into_iter()
        .find(|p| p.server_id == server_id);
    if previous.as_ref().is_some_and(|p| p.host != host) {
        return Err(
            "This peer is already linked at another address. Unlink it before moving the link."
                .into(),
        );
    }
    let previous = previous.map(|peer| peer.credential(profile)).transpose()?;
    let device_id = uuid::Uuid::new_v4().to_string();
    let response = http()?
        .post(format!("http://{address}/v1/pair"))
        .json(&json!({"code":code,"deviceId":device_id,"peer":true}))
        .send()
        .await
        .map_err(|_| "Machine pairing endpoint is unavailable.")?;
    if !response.status().is_success() {
        return Err("Peer pairing refused. Use a fresh shina pair --peer code.".into());
    }
    let issued: Value = response
        .json()
        .await
        .map_err(|_| "Invalid peer pairing response.")?;
    if issued["serverId"] != server_id || issued["peer"] != true {
        return Err("Peer pairing identity or grant mismatch.".into());
    }
    let link = Link {
        server_id: server_id.clone(),
        host: host.into(),
        device_id,
        token: issued["token"]
            .as_str()
            .ok_or("Missing credential.")?
            .into(),
    };
    if let Some(old) = previous {
        let confirmed = match revoke_peer(&old).await {
            Ok(receipt) => receipt["status"] == "accepted",
            Err(error) => error.starts_with("Authentication denied"),
        };
        if !confirmed {
            let _ = revoke_peer(&link).await;
            return Err(
                "The previous peer credential could not be revoked. Its link was kept.".into(),
            );
        }
    }
    let directory = profile.join("peers").join(&server_id);
    let saved = (|| {
        crate::server_auth::protect(&profile.join("peers"))?;
        crate::server_auth::protect(&directory)?;
        crate::server_auth::save_secret(
            &directory.join("credential.json"),
            &serde_json::to_vec(&link).map_err(|_| "Could not encode peer credential.")?,
        )
    })();
    let mut peer = Peer {
        server_id,
        host: host.into(),
        label: identity["label"]
            .as_str()
            .filter(|label| label.len() <= 256)
            .unwrap_or(host)
            .into(),
        linked_at: crate::lifecycle::now(),
        capabilities: json!({}),
        last_reached_at: None,
        last_error: None,
    };
    if saved.is_ok() {
        match crate::client::remote_rpc(&link, "peer.capabilities", json!({})).await {
            Ok(receipt) if receipt["status"] == "accepted" => {
                peer.capabilities = receipt["result"].clone();
                peer.last_reached_at = Some(crate::lifecycle::now());
            }
            _ => peer.last_error = Some("Peer capabilities are unavailable.".into()),
        }
    }
    if let Err(error) = saved.and_then(|_| database.put_peer(&peer)) {
        let _ = revoke_peer(&link).await;
        return Err(error);
    }
    Ok(peer)
}

pub async fn unlink_peer(
    profile: &Path,
    database: &crate::database::Database,
    server: &str,
) -> Result<(), String> {
    let peer = database
        .peers()?
        .into_iter()
        .find(|p| p.server_id == server || p.host == server)
        .ok_or("Unknown peer.")?;
    let link = peer.credential(profile)?;
    let receipt = revoke_peer(&link).await?;
    if receipt["status"] != "accepted" {
        return Err("Revocation was not acknowledged. The peer link was kept.".into());
    }
    database.remove_peer(&peer.server_id)?;
    std::fs::remove_file(
        profile
            .join("peers")
            .join(&peer.server_id)
            .join("credential.json"),
    )
    .map_err(|e| e.to_string())
}

pub struct Machines {
    pub selected: String,
    /// Owner of the sidebar's numeric workspace IDs. The filter belongs to this
    /// window, not to whichever server currently supplies the conversation.
    pub scope_server: Option<String>,
    pub scope_remote: Vec<(String, usize)>,
    pub views: std::collections::BTreeMap<String, Value>,
    pub threads: std::collections::HashMap<String, Vec<crate::model::Thread>>,
    pub ui: Value,
    pub receipts: std::collections::HashMap<String, Vec<crate::client::Event>>,
    pub recover: std::collections::HashMap<String, std::collections::HashMap<usize, uuid::Uuid>>,
    pub project_selection: std::collections::HashMap<String, (usize, String, Option<usize>)>,
    /// The project picker opened by a draft retarget, restricted to this machine.
    pub draft_picker: Option<String>,
    pub picker: bool,
    pub dialog: bool,
    pub host: rune::TextEdit,
    pub code: rune::TextEdit,
    pub peer_host: rune::TextEdit,
    pub peer_code: rune::TextEdit,
    pub notice: Option<String>,
    /// The text of `notice` while it is the unreachable-machines notice, so it clears itself
    /// when the connections are ready again and never clears another message.
    pub auto_notice: Option<String>,
    pub devices: Value,
    pub pending: Option<std::sync::mpsc::Receiver<Result<MachineResult, String>>>,
}
pub enum MachineResult {
    Linked(Link),
    Paired(Value),
    Unlinked(String),
    Devices(Value),
    Peers,
}
impl Default for Machines {
    fn default() -> Self {
        Self {
            selected: "local".into(),
            scope_server: None,
            scope_remote: Vec::new(),
            views: Default::default(),
            threads: Default::default(),
            ui: json!({}),
            receipts: Default::default(),
            recover: Default::default(),
            project_selection: Default::default(),
            draft_picker: None,
            picker: false,
            dialog: false,
            host: Default::default(),
            code: Default::default(),
            peer_host: Default::default(),
            peer_code: Default::default(),
            notice: None,
            auto_notice: None,
            devices: Value::Null,
            pending: None,
        }
    }
}

pub fn tailnet_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(ip) => ip.octets()[0] == 100 && (64..=127).contains(&ip.octets()[1]),
        IpAddr::V6(ip) => ip.segments()[..3] == [0xfd7a, 0x115c, 0xa1e0],
    }
}

/// Fail closed even if a faulty address source supplies a public, LAN or wildcard address.
pub fn bind_addresses(
    source: impl FnOnce() -> Result<Vec<IpAddr>, String>,
) -> Result<Vec<IpAddr>, String> {
    let mut addresses = source()?;
    if addresses.is_empty() || addresses.iter().any(|address| !tailnet_address(*address)) {
        return Err("Tailscale did not report exclusively tailnet addresses. No remote listener was opened.".into());
    }
    addresses.sort();
    addresses.dedup();
    Ok(addresses)
}

/// Where to look for the Tailscale CLI, in order. The macOS app keeps its CLI inside the
/// bundle and launchd services get a minimal PATH, so PATH alone is not enough there.
fn tailscale_candidates() -> Vec<std::path::PathBuf> {
    let mut candidates = vec![std::path::PathBuf::from(if cfg!(windows) {
        "tailscale.exe"
    } else {
        "tailscale"
    })];
    if cfg!(target_os = "macos") {
        candidates.push("/Applications/Tailscale.app/Contents/MacOS/Tailscale".into());
        candidates.push("/opt/homebrew/bin/tailscale".into());
        candidates.push("/usr/local/bin/tailscale".into());
    } else if cfg!(windows) {
        if let Some(programs) = std::env::var_os("ProgramFiles") {
            candidates.push(
                std::path::Path::new(&programs)
                    .join("Tailscale")
                    .join("tailscale.exe"),
            );
        }
    } else {
        candidates.push("/usr/bin/tailscale".into());
        candidates.push("/usr/local/bin/tailscale".into());
    }
    candidates
}

pub fn tailscale_addresses() -> Result<Vec<IpAddr>, String> {
    let output = tailscale_candidates()
        .into_iter()
        .find_map(|executable| {
            let mut command = std::process::Command::new(executable);
            // The macOS app binary opens its GUI instead of acting as the CLI unless told
            // otherwise, which fails under launchd. Other platforms ignore the variable.
            command
                .args(["status", "--json"])
                .env("TAILSCALE_BE_CLI", "1");
            crate::process::output(&mut command, Duration::from_millis(3500), 1024 * 1024)
                .ok()
                .filter(|output| output.status.success())
        })
        .ok_or("Could not read Tailscale status. Check that Tailscale is installed and running.")?;
    if !output.status.success() {
        return Err(
            "Tailscale status failed. Sign in using Tailscale before enabling --tailnet.".into(),
        );
    }
    let status: Value =
        serde_json::from_slice(&output.stdout).map_err(|_| "Invalid Tailscale status JSON.")?;
    status["Self"]["TailscaleIPs"]
        .as_array()
        .ok_or("No Tailscale self addresses.")?
        .iter()
        .map(|value| {
            value
                .as_str()
                .ok_or("Invalid Tailscale address.")?
                .parse()
                .map_err(|_| "Invalid Tailscale address.".into())
        })
        .collect()
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Link {
    pub server_id: String,
    pub host: String,
    pub device_id: String,
    pub token: String,
}

impl Link {
    pub fn directory(&self, profile: &Path) -> PathBuf {
        profile.join("linked").join(&self.server_id)
    }
    pub fn load_all(profile: &Path) -> Result<Vec<Self>, String> {
        let directory = profile.join("linked");
        if !directory.exists() {
            return Ok(vec![]);
        }
        let mut links = Vec::new();
        for entry in std::fs::read_dir(directory).map_err(|e| e.to_string())? {
            let path = entry
                .map_err(|e| e.to_string())?
                .path()
                .join("credential.json");
            if path.is_file() {
                let link: Self =
                    serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
                        .map_err(|_| "Invalid linked credential.")?;
                uuid::Uuid::parse_str(&link.server_id)
                    .map_err(|_| "Invalid linked server identity.")?;
                uuid::Uuid::parse_str(&link.device_id)
                    .map_err(|_| "Invalid linked device identity.")?;
                links.push(link);
            }
        }
        links.sort_by(|a, b| a.server_id.cmp(&b.server_id));
        Ok(links)
    }
    pub fn save(&self, profile: &Path) -> Result<(), String> {
        let directory = self.directory(profile);
        crate::server_auth::protect(&profile.join("linked"))?;
        crate::server_auth::protect(&directory)?;
        crate::server_auth::save_secret(
            &directory.join("credential.json"),
            &serde_json::to_vec(self).map_err(|e| e.to_string())?,
        )
    }
    /// Resolve once and dial the checked numeric address. No proxy or redirect is permitted.
    pub fn address(&self) -> Result<SocketAddr, String> {
        resolve(&self.host)
    }
}

pub fn resolve(host: &str) -> Result<SocketAddr, String> {
    if host.is_empty()
        || host.chars().any(|c| c.is_control() || c.is_whitespace())
        || host.contains(['/', '@', '?', '#', '\\'])
    {
        return Err("Enter a MagicDNS host or tailnet IP, optionally with a port.".into());
    }
    let authority = if host.parse::<IpAddr>().is_ok() {
        format!("{}:{PORT}", bracket(host.parse().expect("checked")))
    } else if host.contains(':') {
        host.to_owned()
    } else {
        format!("{host}:{PORT}")
    };
    let addresses: Vec<_> = authority
        .to_socket_addrs()
        .map_err(|_| "Machine name did not resolve.")?
        .collect();
    if addresses.is_empty()
        || addresses
            .iter()
            .any(|address| !address.ip().is_loopback() && !tailnet_address(address.ip()))
    {
        return Err("The machine must resolve only to loopback or Tailscale addresses.".into());
    }
    Ok(addresses[0])
}

fn bracket(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(ip) => ip.to_string(),
        IpAddr::V6(ip) => format!("[{ip}]"),
    }
}

pub fn http() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|_| "Could not initialize the machine connection.".into())
}

pub async fn link(profile: &Path, host: &str, code: &str) -> Result<Link, String> {
    let previous_links = Link::load_all(profile)?;
    let address = resolve(host)?;
    let device_id = uuid::Uuid::new_v4().to_string();
    let response = http()?
        .post(format!("http://{address}/v1/pair"))
        .json(&json!({"code":code,"deviceId":device_id}))
        .send()
        .await
        .map_err(|_| "Machine pairing endpoint is unavailable.")?;
    if !response.status().is_success() {
        return Err("Pairing refused. The code is unknown, expired, used, or rate-limited.".into());
    }
    let value: Value = response
        .json()
        .await
        .map_err(|_| "Invalid pairing response.")?;
    let server_id = value["serverId"]
        .as_str()
        .ok_or("Missing server identity.")?
        .to_owned();
    uuid::Uuid::parse_str(&server_id).map_err(|_| "Invalid server identity.")?;
    let link = Link {
        server_id,
        host: host.into(),
        device_id,
        token: value["token"]
            .as_str()
            .ok_or("Missing device credential.")?
            .into(),
    };
    if let Some(previous) = previous_links
        .into_iter()
        .find(|previous| previous.server_id == link.server_id)
    {
        let revoked = crate::client::remote_rpc(
            &previous,
            "devices.revoke",
            json!({"deviceId":previous.device_id}),
        )
        .await;
        let confirmed = match revoked {
            Ok(receipt) => receipt["status"] == "accepted",
            Err(error) => error.contains("Authentication denied"),
        };
        if !confirmed {
            let _ = crate::client::remote_rpc(
                &link,
                "devices.revoke",
                json!({"deviceId":link.device_id}),
            )
            .await;
            return Err("The previous link could not be revoked. Its credential was kept. Check the server before pairing again.".into());
        }
    }
    if let Err(error) = link.save(profile) {
        // Do not leave a newly issued credential active if protected storage fails.
        let _ =
            crate::client::remote_rpc(&link, "devices.revoke", json!({"deviceId":link.device_id}))
                .await;
        return Err(error);
    }
    Ok(link)
}

pub async fn unlink(profile: &Path, server: &str) -> Result<(), String> {
    let link = Link::load_all(profile)?
        .into_iter()
        .find(|link| link.server_id == server || link.host == server)
        .ok_or("Unknown linked machine.")?;
    let receipt =
        crate::client::remote_rpc(&link, "devices.revoke", json!({"deviceId":link.device_id}))
            .await?;
    if receipt["status"] != "accepted" {
        return Err("The server did not acknowledge revocation. The link was kept.".into());
    }
    std::fs::remove_file(link.directory(profile).join("credential.json"))
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn cli(args: &[String]) -> Result<(), String> {
    let profile = crate::store::directory().ok_or("No Shina profile.")?;
    let runtime = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
    match args.first().map(String::as_str) {
        Some("pair") if args.len() == 1 || args.len() == 2 && args[1] == "--peer" => {
            let value = runtime.block_on(crate::client::local_rpc(&profile, "devices.pair", json!({"peer":args.len()==2})))?;
            println!("Pairing code: {}\nExpires in 5 minutes. Use once with shina {}link <host> <code>.", value["code"].as_str().ok_or("Missing pairing code.")?,if args.len()==2 {"peer "}else{""});
        }
        Some("peer") => {
            let value = match args.get(1).map(String::as_str) {
                Some("link") if args.len()==4 => runtime.block_on(crate::client::local_rpc(&profile,"peer.link",json!({"host":args[2],"code":args[3]})))?,
                Some("list") if args.len()==2 => runtime.block_on(crate::client::local_rpc(&profile,"peer.list",json!({})))?,
                Some("unlink") if args.len()==3 => runtime.block_on(crate::client::local_rpc(&profile,"peer.unlink",json!({"server":args[2]})))?,
                _ => return Err("Usage: shina peer link <host> <code> | peer list | peer unlink <server-id-or-host>".into()),
            };
            println!("{value}");
        }
        Some("link") if args.len() == 3 => {
            let link = runtime.block_on(link(&profile, &args[1], &args[2]))?;
            println!("Linked {}. Server {}, device {}.", link.host, link.server_id, link.device_id);
        }
        Some("links") if args.len() == 1 => {
            for link in Link::load_all(&profile)? { println!("{}  {}  device {}", link.server_id, link.host, link.device_id); }
        }
        Some("unlink") if args.len() == 2 => {
            runtime.block_on(unlink(&profile, &args[1]))?;
            println!("Revoked the remote credential and removed the local link.");
        }
        Some("devices") if args.len() == 1 => {
            println!("{}", runtime.block_on(crate::client::local_rpc(&profile, "devices.list", json!({})))?);
        }
        Some("devices") if args.len() == 3 && args[1] == "revoke" => {
            let receipt = runtime.block_on(crate::client::local_rpc(&profile, "devices.revoke", json!({"deviceId":args[2]})))?;
            if receipt["status"] != "accepted" { return Err("Device revocation refused.".into()); }
            println!("Device revoked.");
        }
        _ => return Err("Usage: shina pair [--peer] | link <host[:port]> <code> | links | unlink <server-id-or-host> | devices [revoke <device-id>] | peer link <host> <code> | peer list | peer unlink <server-id-or-host>".into()),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn binding_accepts_only_validated_self_tailnet_addresses() {
        for address in [
            "0.0.0.0",
            "::",
            "127.0.0.1",
            "192.168.1.2",
            "8.8.8.8",
            "100.128.0.1",
            "fd00::1",
        ] {
            assert!(bind_addresses(|| Ok(vec![address.parse().unwrap()])).is_err());
        }
        assert!(bind_addresses(|| Ok(vec![])).is_err());
        assert_eq!(
            bind_addresses(|| Ok(vec![
                "100.64.1.2".parse().unwrap(),
                "fd7a:115c:a1e0::1".parse().unwrap()
            ]))
            .unwrap()
            .len(),
            2
        );
    }
}
