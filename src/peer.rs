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

pub struct Machines {
    pub selected: String,
    pub views: std::collections::BTreeMap<String, Value>,
    pub threads: std::collections::HashMap<String, Vec<crate::model::Thread>>,
    pub ui: Value,
    pub receipts: std::collections::HashMap<String, Vec<crate::client::Event>>,
    pub recover: std::collections::HashMap<String, std::collections::HashMap<usize, uuid::Uuid>>,
    pub selection: std::collections::HashMap<String, Option<usize>>,
    pub picker: bool,
    pub dialog: bool,
    pub host: rune::TextEdit,
    pub code: rune::TextEdit,
    pub project_path: rune::TextEdit,
    pub notice: Option<String>,
    pub devices: Value,
    pub pending: Option<std::sync::mpsc::Receiver<Result<MachineResult, String>>>,
}
pub enum MachineResult {
    Linked(Link),
    Paired(Value),
    Unlinked(String),
    Devices(Value),
}
impl Default for Machines {
    fn default() -> Self {
        Self {
            selected: "local".into(),
            views: Default::default(),
            threads: Default::default(),
            ui: json!({}),
            receipts: Default::default(),
            recover: Default::default(),
            selection: Default::default(),
            picker: false,
            dialog: false,
            host: Default::default(),
            code: Default::default(),
            project_path: Default::default(),
            notice: None,
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

pub fn tailscale_addresses() -> Result<Vec<IpAddr>, String> {
    let executable = if cfg!(windows) {
        "tailscale.exe"
    } else {
        "tailscale"
    };
    let output = crate::process::output(
        std::process::Command::new(executable).args(["status", "--json"]),
        Duration::from_millis(3500),
        1024 * 1024,
    )
    .map_err(
        |_| "Could not read Tailscale status. Check that Tailscale is installed and running.",
    )?;
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
        Some("pair") if args.len() == 1 => {
            let value = runtime.block_on(crate::client::local_rpc(&profile, "devices.pair", json!({})))?;
            println!("Pairing code: {}\nExpires in 5 minutes. Use once with shina link <host> <code>.", value["code"].as_str().ok_or("Missing pairing code.")?);
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
        _ => return Err("Usage: shina pair | link <host[:port]> <code> | links | unlink <server-id-or-host> | devices [revoke <device-id>]".into()),
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
