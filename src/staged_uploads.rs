//! KSG's pending attachment store and claimPendingAttachments. Never delete a claimed blob.
use crate::attachments::{self, Attachment};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

pub const RETENTION_SECONDS: u64 = 24 * 60 * 60;

#[derive(Deserialize, Serialize)]
pub struct Lease {
    pub device: String,
    pub expires: u64,
    pub attachment: Attachment,
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn directory(profile: &Path, id: &str) -> Result<PathBuf, String> {
    uuid::Uuid::parse_str(id).map_err(|_| "Invalid upload identity.")?;
    Ok(profile.join("uploads").join("staged").join(id))
}

pub fn publish(
    profile: &Path,
    id: &str,
    device: &str,
    name: &str,
    mime: Option<&str>,
    bytes: &[u8],
) -> Result<serde_json::Value, String> {
    let directory = directory(profile, id)?;
    let attachment = if let Some(mime) = mime {
        attachments::store_uploaded_file(&directory, name, mime, bytes)?
    } else {
        attachments::store_uploaded(&directory, name, bytes)?
    };
    let lease = Lease {
        device: device.into(),
        expires: now() + RETENTION_SECONDS,
        attachment,
    };
    crate::store::write_atomic(
        &directory.join("lease.json"),
        &serde_json::to_vec(&lease).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let mut value = lease.attachment.to_value();
    value["stagedUploadId"] = serde_json::json!(id);
    Ok(value)
}

pub fn read(profile: &Path, id: &str, device: &str) -> Result<Lease, String> {
    let directory = directory(profile, id)?;
    let file = fs::File::open(directory.join("lease.json"))
        .map_err(|_| "Uploaded file expired. Remove it and attach it again.")?;
    let lease: Lease =
        serde_json::from_reader(file.take(8192)).map_err(|_| "Invalid staged upload.")?;
    if lease.device != device || lease.expires <= now() {
        return Err("Uploaded file expired. Remove it and attach it again.".into());
    }
    if !(attachments::safe_file_id(&lease.attachment.id)
        || attachments::safe_id(&lease.attachment.id))
    {
        return Err("Invalid staged attachment identity.".into());
    }
    let metadata = fs::symlink_metadata(directory.join(&lease.attachment.id))
        .map_err(|_| "The original attachment is no longer available.")?;
    if !metadata.is_file() || metadata.len() != lease.attachment.bytes {
        return Err("The original attachment is no longer available.".into());
    }
    Ok(lease)
}

pub fn claim(profile: &Path, id: &str, device: &str) -> Result<Attachment, String> {
    let lease = read(profile, id, device)?;
    let path = directory(profile, id)?.join(&lease.attachment.id);
    let maximum = if lease.attachment.is_image() {
        attachments::MAX_IMAGE_BYTES
    } else {
        attachments::MAX_FILE_BYTES
    };
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(maximum + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 != lease.attachment.bytes || bytes.len() as u64 > maximum {
        return Err("Attachment stored size does not match.".into());
    }
    // Copy-on-claim preserves the pending source for a retry. Content addressing lets
    // Shina share an immutable claimed blob; releasing a pending lease cannot erase it.
    let claimed = if lease.attachment.is_image() {
        attachments::store_uploaded(&profile.join("attachments"), &lease.attachment.name, &bytes)
    } else {
        attachments::store_uploaded_file(
            &profile.join("attachments"),
            &lease.attachment.name,
            &lease.attachment.mime,
            &bytes,
        )
    }?;
    if claimed.id != lease.attachment.id {
        return Err("Attachment hash mismatch.".into());
    }
    Ok(claimed)
}

pub fn blob_page(
    profile: &Path,
    id: &str,
    device: &str,
    offset: u64,
    limit: usize,
) -> Result<(Vec<u8>, u64), String> {
    use std::io::Seek;
    let lease = read(profile, id, device)?;
    let mut file = fs::File::open(directory(profile, id)?.join(&lease.attachment.id))
        .map_err(|e| e.to_string())?;
    if offset > lease.attachment.bytes {
        return Err("Invalid blob offset.".into());
    }
    file.seek(std::io::SeekFrom::Start(offset))
        .map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(limit.clamp(1, crate::protocol::MAX_PAGE_BYTES) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    Ok((bytes, lease.attachment.bytes))
}

pub fn release(profile: &Path, id: &str, device: &str) -> Result<(), String> {
    let lease = read(profile, id, device)?;
    if lease.device != device {
        return Err("Upload unavailable.".into());
    }
    fs::remove_dir_all(directory(profile, id)?).map_err(|e| e.to_string())
}

pub fn sweep(profile: &Path) {
    let root = profile.join("uploads").join("staged");
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(id) = entry.file_name().into_string() else {
            continue;
        };
        if uuid::Uuid::parse_str(&id).is_err() {
            continue;
        }
        let Ok(metadata) = fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if !metadata.is_dir() {
            continue;
        }
        let expired = fs::read(entry.path().join("lease.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Lease>(&bytes).ok())
            .map_or_else(
                || {
                    metadata
                        .modified()
                        .ok()
                        .and_then(|time| time.elapsed().ok())
                        .is_some_and(|age| age.as_secs() >= RETENTION_SECONDS)
                },
                |lease| lease.expires <= now(),
            );
        if expired {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn claims_preserve_pending_sources_release_never_erases_claims_and_owner_is_checked() {
        let profile = crate::store::directory().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        publish(
            &profile,
            &id,
            "owner",
            "file.txt",
            Some("text/plain"),
            b"original",
        )
        .unwrap();
        assert!(read(&profile, &id, "other").is_err());
        let first = claim(&profile, &id, "owner").unwrap();
        assert_eq!(claim(&profile, &id, "owner").unwrap(), first);
        release(&profile, &id, "owner").unwrap();
        assert_eq!(fs::read(first.path.unwrap()).unwrap(), b"original");
        assert!(read(&profile, &id, "owner").is_err());
    }
}
