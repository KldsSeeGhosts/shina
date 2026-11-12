//! Local bootstrap credentials. No credential is passed in argv or logged.
use serde::{Deserialize, Serialize};
use std::path::Path;

/// One live code per host, with a bounded global attempt window. Restart invalidates it.
#[derive(Default)]
pub struct Pairing {
    code_hash: String,
    expires: u64,
    deadline: Option<std::time::Instant>,
    attempts: u32,
    window: u64,
    peer: bool,
}
impl Pairing {
    pub fn is_peer(&self) -> bool { self.peer }
    #[cfg(test)]
    pub fn create(&mut self, now: u64) -> Result<serde_json::Value, String> {
        self.create_scoped(now, false)
    }
    pub fn create_scoped(&mut self, now: u64, peer: bool) -> Result<serde_json::Value, String> {
        if self.expires > now
            && self
                .deadline
                .is_some_and(|deadline| std::time::Instant::now() < deadline)
        {
            return Err(
                "A pairing code is already active. Wait for its five-minute expiry.".into(),
            );
        }
        let code = secret()[..12].to_ascii_uppercase();
        self.code_hash = crate::protocol::revision(&serde_json::json!(code));
        self.peer = peer;
        self.expires = now + 300_000;
        self.deadline = Some(std::time::Instant::now() + std::time::Duration::from_secs(300));
        Ok(serde_json::json!({"code":code,"expiresAt":self.expires}))
    }
    pub fn exchange(
        &mut self,
        database: &crate::database::Database,
        code: &str,
        device: &str,
        now: u64,
    ) -> Result<String, String> {
        if now.saturating_sub(self.window) >= 60_000 {
            self.window = now;
            self.attempts = 0;
        }
        self.attempts = self.attempts.saturating_add(1);
        if self.attempts > 5
            || now >= self.expires
            || self
                .deadline
                .is_none_or(|deadline| std::time::Instant::now() >= deadline)
            || self.code_hash.is_empty()
            || code.len() != 12
            || self.code_hash != crate::protocol::revision(&serde_json::json!(code))
            || uuid::Uuid::parse_str(device).is_err()
        {
            return Err("Pairing refused.".into());
        }
        // Consume before issuing. Storage failure never makes a code reusable.
        self.code_hash.clear();
        self.expires = 0;
        let token = secret();
        let scopes: &[&str] = if self.peer { &["read", "peer-delegate"] } else { &["paired", "read", "operate", "approve"] };
        database.issue_scoped_device(device, &token, scopes)?;
        Ok(token)
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Discovery {
    pub server_id: String,
    pub instance_id: String,
    pub epoch: String,
    pub pid: u32,
    pub started_at: u64,
    pub port: u16,
    pub protocol_major: u32,
    pub protocol_minor: u32,
    pub binary_version: String,
    pub bootstrap_token: String,
}

pub fn secret() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("OS randomness");
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn protect(directory: &Path) -> Result<(), String> {
    std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    #[cfg(windows)]
    {
        // Build a fresh DACL, not /grant:r, which would retain other explicit grants.
        let identity = crate::process::output(
            std::process::Command::new("whoami.exe").args(["/user", "/fo", "csv", "/nh"]),
            std::time::Duration::from_secs(5),
            8192,
        )
        .map_err(|e| e.to_string())?;
        if !identity.status.success() {
            return Err("Could not resolve the current user's SID.".into());
        }
        let output = String::from_utf8_lossy(&identity.stdout);
        let sid = output
            .split('"')
            .find(|part| part.starts_with("S-1-"))
            .ok_or("Missing user SID.")?;
        restrict_windows(directory, sid, true)?;
        for name in [
            "server.json",
            "server.json.tmp",
            "credential.json",
            "credential.json.tmp",
            "client-credential.json",
            "client-credential.json.tmp",
        ] {
            let path = directory.join(name);
            if path.exists() {
                restrict_windows(&path, sid, false)?;
            }
        }
    }
    Ok(())
}

pub fn save_secret(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let directory = path.parent().ok_or("Missing credential directory.")?;
    protect(directory)?;
    crate::store::write_atomic(path, bytes).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(windows)]
fn restrict_windows(path: &Path, sid: &str, directory: bool) -> Result<(), String> {
    use std::{ffi::c_void, os::windows::ffi::OsStrExt};
    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
            text: *const u16,
            revision: u32,
            descriptor: *mut *mut c_void,
            size: *mut u32,
        ) -> i32;
        fn SetFileSecurityW(path: *const u16, information: u32, descriptor: *const c_void) -> i32;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LocalFree(memory: *mut c_void) -> *mut c_void;
    }
    let inherit = if directory { "OICI" } else { "" };
    let descriptor: Vec<u16> = format!("D:P(A;{inherit};FA;;;{sid})")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut security = std::ptr::null_mut();
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            descriptor.as_ptr(),
            1,
            &mut security,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(format!(
            "Could not build the user-only server ACL: {}",
            std::io::Error::last_os_error()
        ));
    }
    // DACL only. Do not request SACL/owner changes or SeSecurityPrivilege.
    let result = unsafe { SetFileSecurityW(path.as_ptr(), 0x80000004, security) };
    let error = std::io::Error::last_os_error();
    unsafe {
        LocalFree(security);
    }
    if result == 0 {
        return Err(format!("Could not restrict the server ACL: {error}"));
    }
    Ok(())
}

impl Discovery {
    pub fn read(directory: &Path) -> Result<Self, String> {
        serde_json::from_slice(
            &std::fs::read(directory.join("server.json")).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())
    }
    pub fn publish(&self, directory: &Path) -> Result<(), String> {
        crate::store::write_atomic(
            &directory.join("server.json"),
            &serde_json::to_vec(self).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                directory.join("server.json"),
                std::fs::Permissions::from_mode(0o600),
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}
