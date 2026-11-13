//! Workspace entry search behind `workspace.searchEntries`.
//!
//! Ported from KSG Code's `WorkspaceSearchIndex`/`WorkspaceEntries.search`. KSG traverses the
//! workspace with fff (a directory walk honoring .gitignore/.ignore/.git exclude) capped at
//! 25,000 indexed entries and a 15-second scan budget. This port uses `ignore::WalkBuilder`
//! for the same traversal semantics and KSG's `scoreQueryMatch`/`scoreSubsequenceMatch` ranking
//! (the shared ranking already ported in `crate::composer`).
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// KSG `WORKSPACE_INDEX_MAX_ENTRIES`.
pub const MAX_INDEX_ENTRIES: usize = 25_000;
/// KSG's 15-second scan timeout.
pub const SCAN_BUDGET: Duration = Duration::from_secs(15);
/// Contract cap mirroring KSG's `PROJECT_SEARCH_ENTRIES_MAX_LIMIT`.
pub const MAX_LIMIT: usize = 500;

pub const IMAGE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "avif", "svg", "ico", "tiff",
];

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceEntry {
    /// Path relative to the searched `cwd`, POSIX separators.
    pub path: String,
    /// `file` or `directory`.
    pub kind: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ignored: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSearchResult {
    pub entries: Vec<WorkspaceEntry>,
    pub truncated: bool,
}

#[derive(Clone, Debug)]
struct Indexed {
    relative: String,
    lowered: String,
    name: String,
    kind: &'static str,
}

fn normalize_query(query: &str) -> String {
    // KSG `normalizeSearchQuery(query, { trimLeadingPattern: /^[@./]+/ })`.
    query
        .trim()
        .trim_start_matches(['@', '.', '/'])
        .to_lowercase()
}

fn is_image(relative: &str) -> bool {
    Path::new(relative)
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| IMAGE_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
}

fn index(cwd: &Path) -> (Vec<Indexed>, bool) {
    let started = Instant::now();
    let mut entries = Vec::new();
    let mut truncated = false;
    let walker = ignore::WalkBuilder::new(cwd)
        .hidden(true)
        .git_ignore(true)
        .git_exclude(true)
        .git_global(true)
        .ignore(true)
        .parents(true)
        .require_git(false)
        .follow_links(false)
        .build();
    for result in walker {
        if result.is_err() {
            continue;
        }
        let entry = result.unwrap();
        if entry.path() == cwd {
            continue;
        }
        let is_dir = entry.file_type().is_some_and(|kind| kind.is_dir());
        let is_file = entry.file_type().is_some_and(|kind| kind.is_file());
        if !is_dir && !is_file {
            continue;
        }
        let Ok(relative) = entry.path().strip_prefix(cwd) else {
            continue;
        };
        let relative = relative
            .components()
            .map(|part| part.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        if relative.is_empty() {
            continue;
        }
        let name = relative
            .rsplit('/')
            .next()
            .unwrap_or(&relative)
            .to_lowercase();
        entries.push(Indexed {
            lowered: relative.to_lowercase(),
            name,
            relative,
            kind: if is_dir { "directory" } else { "file" },
        });
        if entries.len() >= MAX_INDEX_ENTRIES || started.elapsed() >= SCAN_BUDGET {
            truncated = true;
            break;
        }
    }
    (entries, truncated)
}

/// KSG's candidate ordering: the best `scoreQueryMatch` across the full relative path and the
/// entry name, ascending score then path.
fn score_entry(entry: &Indexed, query: &str) -> Option<usize> {
    const MARKERS: &[&str] = &["/", "-", "_", " ", "."];
    let path_score = crate::composer::score_query_match(
        &entry.lowered,
        query,
        0,
        Some(2),
        Some(4),
        Some(6),
        Some(100),
        MARKERS,
    );
    let name_score = crate::composer::score_query_match(
        &entry.name,
        query,
        0,
        Some(1),
        Some(3),
        Some(5),
        Some(100),
        MARKERS,
    );
    [path_score, name_score].into_iter().flatten().min()
}

/// `cwd` must be an existing absolute directory that is a known project root or inside one.
pub fn resolve_cwd(cwd: &str, project_roots: &[PathBuf]) -> Result<PathBuf, String> {
    if cwd.trim().is_empty() || !Path::new(cwd).is_absolute() {
        return Err("Workspace path must be absolute.".into());
    }
    let path = Path::new(cwd);
    if !path.is_dir() {
        return Err("Workspace path is not a directory.".into());
    }
    let canonical = path
        .canonicalize()
        .map_err(|_| "Workspace path could not be resolved.".to_string())?;
    let inside = project_roots.iter().any(|root| {
        root.canonicalize()
            .is_ok_and(|root| canonical == root || canonical.starts_with(&root))
    });
    if inside {
        Ok(canonical)
    } else {
        Err("Workspace path is not inside a project on this server.".into())
    }
}

/// `workspace.readFile` path rule: after following symlinks, the file must land
/// inside a known project root, the same check `resolve_cwd` applies, or be one
/// of the skill instruction paths this server reported. A relative `path`
/// resolves against `cwd`, which must itself sit inside a project.
pub fn resolve_read(
    cwd: &str,
    path: &str,
    project_roots: &[PathBuf],
    skill_paths: &[PathBuf],
) -> Result<PathBuf, String> {
    let raw = Path::new(path);
    let joined = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        resolve_cwd(cwd, project_roots)?.join(raw)
    };
    let canonical = joined
        .canonicalize()
        .map_err(|_| "Workspace file could not be resolved.".to_string())?;
    let inside = project_roots.iter().any(|root| {
        root.canonicalize()
            .is_ok_and(|root| canonical == root || canonical.starts_with(&root))
    });
    if inside {
        return Ok(canonical);
    }
    let reported = skill_paths
        .iter()
        .any(|skill| skill.canonicalize().is_ok_and(|skill| canonical == skill));
    if reported {
        Ok(canonical)
    } else {
        Err("Workspace path is not inside a project on this server.".into())
    }
}

pub fn search(
    cwd: &Path,
    query: &str,
    limit: usize,
    kind: Option<&str>,
    image_only: bool,
) -> WorkspaceSearchResult {
    let (indexed, mut truncated) = index(cwd);
    let limit = limit.clamp(1, MAX_LIMIT);
    // KSG filters by kind/image before the limit so non-matches do not consume the page.
    let filtered = indexed
        .into_iter()
        .filter(|entry| kind.is_none_or(|kind| entry.kind == kind))
        .filter(|entry| !image_only || is_image(&entry.relative));
    let normalized = normalize_query(query);
    let mut ranked: Vec<(usize, Indexed)> = if normalized.is_empty() {
        filtered
            .map(|entry| {
                let depth = entry.relative.matches('/').count();
                (depth * 1000, entry)
            })
            .collect()
    } else {
        filtered
            .filter_map(|entry| score_entry(&entry, &normalized).map(|score| (score, entry)))
            .collect()
    };
    ranked.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.relative.cmp(&b.1.relative)));
    if ranked.len() > limit {
        ranked.truncate(limit);
        truncated = true;
    }
    WorkspaceSearchResult {
        entries: ranked
            .into_iter()
            .map(|(_, entry)| WorkspaceEntry {
                path: entry.relative.clone(),
                kind: entry.kind.to_owned(),
                ignored: false,
            })
            .collect(),
        truncated,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn project(entries: &[&str]) -> PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "shina-ws-search-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        for entry in entries {
            let path = root.join(entry);
            if entry.ends_with('/') {
                fs::create_dir_all(&path).unwrap();
            } else {
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(&path, "x").unwrap();
            }
        }
        root
    }

    #[test]
    fn finds_nested_files_and_folders_and_obeys_gitignore() {
        let root = project(&[
            "src/deep/nested_file.rs",
            "src/folder_marker/inner.rs",
            "ignored_secret.txt",
        ]);
        fs::write(root.join(".gitignore"), "ignored_secret.txt\n").unwrap();
        let roots = [root.clone()];
        let cwd = resolve_cwd(root.to_str().unwrap(), &roots).unwrap();
        let found = search(&cwd, "nested", 50, None, false);
        assert!(
            found
                .entries
                .iter()
                .any(|e| e.path == "src/deep/nested_file.rs" && e.kind == "file")
        );
        let folder = search(&cwd, "folder_marker", 50, Some("directory"), false);
        assert!(
            folder
                .entries
                .iter()
                .any(|e| e.path == "src/folder_marker" && e.kind == "directory")
        );
        assert!(
            search(&cwd, "ignored_secret", 50, None, false)
                .entries
                .is_empty()
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn resolve_read_stays_inside_projects_or_a_reported_skill_path() {
        let root = project(&["src/deep.rs", "README.md"]);
        let outside_dir = project(&["escape.txt"]);
        let roots = vec![root.clone()];
        let cwd = root.to_string_lossy().into_owned();
        // Relative and absolute project paths resolve to their canonical form.
        assert_eq!(
            resolve_read(&cwd, "src/deep.rs", &roots, &[]).unwrap(),
            root.join("src/deep.rs").canonicalize().unwrap()
        );
        let absolute = root.join("README.md").to_string_lossy().into_owned();
        assert!(resolve_read("", &absolute, &roots, &[]).is_ok());
        // A `..` escape and an absolute path outside every root are refused.
        let escape = root.join("../escape.txt").to_string_lossy().into_owned();
        assert!(
            resolve_read(&cwd, "../escape.txt", &roots, &[]).is_err(),
            "{escape}"
        );
        assert!(resolve_read(&cwd, &escape, &roots, &[]).is_err());
        let foreign = outside_dir
            .join("escape.txt")
            .to_string_lossy()
            .into_owned();
        assert!(resolve_read("", &foreign, &roots, &[]).is_err());
        // Missing files do not resolve at all.
        assert!(resolve_read(&cwd, "missing.rs", &roots, &[]).is_err());
        // A skill path outside the roots is allowed only when the server's own
        // skill inventory reported it.
        assert!(resolve_read("", &foreign, &roots, &[outside_dir.join("escape.txt")]).is_ok());
        assert!(resolve_read("", &foreign, &roots, &[outside_dir.join("other.txt")]).is_err());
        #[cfg(unix)]
        {
            // A symlink inside the project resolving outside it is refused.
            let link = root.join("linked.txt");
            std::os::unix::fs::symlink(outside_dir.join("escape.txt"), &link).unwrap();
            let via_link = link.to_string_lossy().into_owned();
            assert!(
                resolve_read(&cwd, "linked.txt", &roots, &[]).is_err(),
                "{via_link}"
            );
            let _ = fs::remove_file(&link);
        }
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&outside_dir);
    }

    #[test]
    fn honors_limit_and_marks_truncated() {
        let names: Vec<String> = (0..30).map(|i| format!("match_{i:02}.rs")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let root = project(&refs);
        let roots = [root.clone()];
        let cwd = resolve_cwd(root.to_str().unwrap(), &roots).unwrap();
        let found = search(&cwd, "match", 5, None, false);
        assert_eq!(found.entries.len(), 5);
        assert!(found.truncated);
    }

    #[test]
    fn rejects_cwd_outside_any_project_and_never_escapes_cwd() {
        let outside = project(&["escape.txt"]);
        let root = project(&["src/inside.rs"]);
        let roots = [root.clone()];
        assert!(resolve_cwd(outside.to_str().unwrap(), &roots).is_err());
        assert!(resolve_cwd(root.join("..").to_str().unwrap(), &roots).is_err());
        let sub = root.join("src");
        let cwd = resolve_cwd(sub.to_str().unwrap(), &roots).unwrap();
        let found = search(&cwd, "", 50, None, false);
        assert_eq!(found.entries.len(), 1);
        assert_eq!(found.entries[0].path, "inside.rs");
        assert!(
            found
                .entries
                .iter()
                .all(|e| !e.path.starts_with("..") && !Path::new(&e.path).is_absolute())
        );
    }

    #[test]
    fn image_filter_applies_before_the_limit() {
        // Copied from KSG's WorkspaceSearchIndex image-filter case.
        let root = project(&["note.md", "pic_one.png", "pic_two.png"]);
        let roots = [root.clone()];
        let cwd = resolve_cwd(root.to_str().unwrap(), &roots).unwrap();
        let found = search(&cwd, "", 1, None, true);
        assert_eq!(found.entries.len(), 1);
        assert!(found.entries[0].path.ends_with(".png"));
    }

    #[test]
    fn literal_bracket_characters_match() {
        // KSG's fff treats brackets literally.
        let root = project(&["docs/[draft] plan.md", "docs/other.md"]);
        let roots = [root.clone()];
        let cwd = resolve_cwd(root.to_str().unwrap(), &roots).unwrap();
        let found = search(&cwd, "[draft]", 50, None, false);
        assert_eq!(found.entries.len(), 1);
        assert_eq!(found.entries[0].path, "docs/[draft] plan.md");
    }

    #[test]
    fn exact_and_prefix_matches_rank_before_subsequence() {
        // KSG scoreQueryMatch ordering: exact < prefix < boundary < includes < fuzzy.
        let root = project(&["model.rs", "src/model_picker.rs", "src/m_older_delta.rs"]);
        let roots = [root.clone()];
        let cwd = resolve_cwd(root.to_str().unwrap(), &roots).unwrap();
        let found = search(&cwd, "model", 50, None, false);
        assert_eq!(found.entries[0].path, "model.rs");
        assert_eq!(found.entries[1].path, "src/model_picker.rs");
    }
}
