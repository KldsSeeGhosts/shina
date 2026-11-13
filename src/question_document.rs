//! KSG DocumentText's isolated and bounded document extraction.
use std::{
    collections::{HashMap, VecDeque},
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Mutex, OnceLock, mpsc},
    time::{Duration, Instant},
};

enum Failure {
    Permanent,
    RetryAt(Instant),
}
#[derive(Default)]
struct Failures {
    values: HashMap<PathBuf, Failure>,
    order: VecDeque<PathBuf>,
}
static FAILURES: OnceLock<Mutex<Failures>> = OnceLock::new();

pub fn ensure_named(path: &Path, name: &str) -> Option<PathBuf> {
    let extension = Path::new(name).extension()?.to_str()?.to_lowercase();
    if ![
        "docx", "xlsx", "pptx", "odt", "ods", "odp", "rtf", "epub", "ipynb",
    ]
    .contains(&extension.as_str())
    {
        return None;
    }
    if std::fs::metadata(path).ok()?.len() > 16 * 1024 * 1024 {
        return None;
    }
    if path
        .extension()
        .is_some_and(|value| value == extension.as_str())
    {
        return ensure(path);
    }
    // Shina's immutable file blobs use .bin. Keep a typed, owned extraction view.
    let root = path.parent()?.join(".document-text");
    std::fs::create_dir_all(&root).ok()?;
    let typed = root.join(format!("{}.{}", path.file_name()?.to_str()?, extension));
    if !typed.is_file() && std::fs::hard_link(path, &typed).is_err() {
        std::fs::copy(path, &typed).ok()?;
    }
    ensure(&typed)
}

pub fn prepare_answer_attachments(
    params: &serde_json::Value,
    directory: &Path,
) -> Result<(), String> {
    let by_question = params["attachmentsByQuestionId"]
        .as_object()
        .ok_or("Invalid question attachments.")?;
    if by_question.values().any(|v| {
        v.as_array()
            .is_none_or(|a| a.len() > crate::attachments::MAX_ATTACHMENTS)
    }) {
        return Err("Invalid question attachments.".into());
    }
    for value in params["attachmentsByQuestionId"]
        .as_object()
        .into_iter()
        .flat_map(|m| m.values())
        .flat_map(|v| v.as_array().into_iter().flatten())
    {
        let id = value["id"].as_str().ok_or("Invalid answer attachment.")?;
        if !crate::attachments::safe_id(id) && !crate::attachments::safe_file_id(id) {
            return Err("Answer attachments must be uploaded first.".into());
        }
        let root = directory
            .join("attachments")
            .canonicalize()
            .map_err(|e| e.to_string())?;
        let path = root
            .join(id)
            .canonicalize()
            .map_err(|_| "The answer attachment is no longer available.")?;
        if !path.starts_with(root) {
            return Err("Invalid answer attachment path.".into());
        }
        ensure_named(&path, value["name"].as_str().unwrap_or(""));
    }
    Ok(())
}

pub fn ensure(path: &Path) -> Option<PathBuf> {
    let extension = path.extension()?.to_str()?.to_lowercase();
    if ![
        "docx", "xlsx", "pptx", "odt", "ods", "odp", "rtf", "epub", "ipynb",
    ]
    .contains(&extension.as_str())
    {
        return None;
    }
    let sidecar = PathBuf::from(format!("{}.txt", path.to_string_lossy()));
    if sidecar.is_file() {
        return Some(sidecar);
    }
    // One extraction at a time, as KSG DocumentText's semaphore.
    let mut failed = FAILURES.get_or_init(Default::default).lock().ok()?;
    if sidecar.is_file() {
        return Some(sidecar);
    }
    if failed
        .values
        .get(path)
        .is_some_and(|failure| match failure {
            Failure::Permanent => true,
            Failure::RetryAt(at) => *at > Instant::now(),
        })
    {
        return None;
    }
    if std::fs::metadata(path).ok()?.len() > 16 * 1024 * 1024 {
        return None;
    }
    let result = (|| -> Result<String, bool> {
        let worker = path
            .parent()
            .ok_or(false)?
            .join(".shina-question-document-worker.cjs");
        let source = format!(
            "/*\n{}\n*/\n{}",
            include_str!("../assets/question_document-LICENSE.txt"),
            include_str!("../assets/question_document_worker.cjs")
        );
        if std::fs::read(&worker).ok().as_deref() != Some(source.as_bytes()) {
            std::fs::write(&worker, &source).map_err(|_| false)?;
        }
        let mut command = Command::new("node");
        command
            .arg("--max-old-space-size=192")
            .arg(worker)
            .arg(path)
            .arg(format!(".{extension}"))
            .env("ELECTRON_RUN_AS_NODE", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut process = command.spawn().map_err(|_| false)?;
        let stdout = process.stdout.take().ok_or(false)?;
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let result = stdout
                .take(4 * 1024 * 1024 + 1025)
                .read_to_end(&mut bytes)
                .map(|_| bytes);
            let _ = tx.send(result);
        });
        let deadline = Instant::now() + Duration::from_secs(15);
        let status = loop {
            if let Some(status) = process.try_wait().map_err(|_| false)? {
                break status;
            }
            if Instant::now() >= deadline {
                let _ = process.kill();
                let _ = process.wait();
                return Err(false);
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        let bytes = rx
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| false)?
            .map_err(|_| false)?;
        if !status.success() {
            return Err(status.code() == Some(2));
        }
        if bytes.len() > 4 * 1024 * 1024 + 1024 {
            return Err(false);
        }
        String::from_utf8(bytes).map_err(|_| false)
    })();
    match result {
        Ok(content) => {
            let pending = PathBuf::from(format!("{}.part", sidecar.to_string_lossy()));
            let result =
                std::fs::write(&pending, content).and_then(|_| std::fs::rename(&pending, &sidecar));
            let _ = std::fs::remove_file(&pending);
            result.ok().map(|_| sidecar)
        }
        Err(permanent) => {
            if failed.values.len() >= 1024
                && let Some(key) = failed.order.pop_front()
            {
                failed.values.remove(&key);
            }
            if !failed.values.contains_key(path) {
                failed.order.push_back(path.to_owned());
            }
            failed.values.insert(
                path.to_owned(),
                if permanent {
                    Failure::Permanent
                } else {
                    Failure::RetryAt(Instant::now() + Duration::from_secs(1))
                },
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn supported_notebooks_extract_once_to_a_reusable_sidecar() {
        let root =
            std::env::temp_dir().join(format!("shina-question-document-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("answer.ipynb");
        std::fs::write(
            &path,
            r##"{"cells":[{"cell_type":"markdown","source":["# Answer\n","Use disk."]}]}"##,
        )
        .unwrap();
        let sidecar = ensure(&path).expect("extracted notebook");
        assert_eq!(
            std::fs::read_to_string(&sidecar).unwrap(),
            "## Cell 1 (markdown)\n# Answer\nUse disk."
        );
        std::fs::write(&path, "invalid notebook after successful extraction").unwrap();
        assert_eq!(ensure(&path), Some(sidecar));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn rejected_documents_remain_rejected_and_unsupported_files_do_not_extract() {
        let root = std::env::temp_dir().join(format!(
            "shina-question-document-rejected-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("invalid.ipynb");
        std::fs::write(&path, "not JSON").unwrap();
        assert!(ensure(&path).is_none());
        std::fs::write(&path, r#"{"cells":[{"source":"now valid"}]}"#).unwrap();
        assert!(
            ensure(&path).is_none(),
            "exit 2 is a permanent rejection for this path"
        );
        let unsupported = root.join("plain.txt");
        std::fs::write(&unsupported, "Already plain text").unwrap();
        assert!(ensure(&unsupported).is_none());
        std::fs::remove_dir_all(root).unwrap();
    }
}
