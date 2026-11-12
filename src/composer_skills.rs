//! Skill inventories and invocation forms ported from KSG Code's provider commands,
//! CodexProvider, ACP registry probe, ClaudeSkills and ClaudeSkillDispatch.
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::Read,
    path::{Path, PathBuf},
};

const LIMIT: usize = 1024;
const READ_LIMIT: u64 = 256 * 1024;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub path: Option<PathBuf>,
    pub user_only: bool,
}

pub fn from_commands(commands: &[Value]) -> Vec<Skill> {
    let mut skills = BTreeMap::new();
    for command in commands.iter().take(4096) {
        let Some(raw) = command["name"].as_str() else {
            continue;
        };
        let name = raw
            .strip_prefix("skill:")
            .or_else(|| raw.strip_prefix('$'))
            .or_else(|| (command["source"] == "skill").then_some(raw));
        let Some(name) = name.filter(|name| valid_name(name)) else {
            continue;
        };
        if command["enabled"] == false || command["userInvocable"] == false {
            continue;
        }
        let description = command["shortDescription"]
            .as_str()
            .or_else(|| command["description"].as_str())
            .unwrap_or("");
        skills.entry(name.to_owned()).or_insert_with(|| Skill {
            name: name.into(),
            description: description.chars().take(1000).collect(),
            path: command["sourceInfo"]["path"]
                .as_str()
                .or_else(|| command["path"].as_str())
                .map(PathBuf::from),
            user_only: command["userInvocationOnly"] == true,
        });
    }
    skills.into_values().take(LIMIT).collect()
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 256
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b":_-".contains(&c))
        && name.bytes().any(|c| c.is_ascii_alphabetic())
}

pub fn codex_commands(result: &Value, cwd: &Path) -> Vec<Value> {
    let data = result["data"].as_array().cloned().unwrap_or_default();
    let matching = data
        .iter()
        .find(|entry| entry["cwd"].as_str().is_some_and(|p| Path::new(p) == cwd));
    let entries: Vec<_> = matching.map_or_else(|| data.iter().collect(), |entry| vec![entry]);
    let mut seen = std::collections::HashSet::new();
    entries.into_iter().flat_map(|entry| entry["skills"].as_array().into_iter().flatten())
        .filter(|skill| skill["enabled"] != false)
        .filter_map(|skill| {
            let name = skill["name"].as_str().filter(|name| valid_name(name))?;
            if !seen.insert(name.to_owned()) { return None; }
            Some(json!({"name":format!("${name}"),"description":
                skill["shortDescription"].as_str().or_else(|| skill["interface"]["shortDescription"].as_str())
                    .or_else(|| skill["description"].as_str()).unwrap_or(""),
                "path":skill["path"],"source":"Codex"}))
        }).take(LIMIT).collect()
}

pub fn is_claude(agent: &crate::harness::Agent) -> bool {
    agent.id.contains("claude")
        || agent.command.to_ascii_lowercase().contains("claude")
        || agent
            .args
            .iter()
            .any(|arg| arg.to_ascii_lowercase().contains("claude-code-acp"))
}

fn read(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(READ_LIMIT + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > READ_LIMIT {
        return None;
    }
    String::from_utf8(bytes).ok()
}

fn json_file(path: &Path) -> Value {
    read(path)
        .and_then(|text| serde_json::from_str(&jsonc(&text)).ok())
        .unwrap_or(Value::Null)
}

/// Claude's settings allow comments and trailing commas. Preserve strings and their escapes.
fn jsonc(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut clean = Vec::with_capacity(bytes.len());
    let (mut at, mut quoted, mut escaped) = (0, false, false);
    while at < bytes.len() {
        let c = bytes[at];
        if quoted {
            clean.push(c);
            if escaped {
                escaped = false;
            } else if c == b'\\' {
                escaped = true;
            } else if c == b'"' {
                quoted = false;
            }
        } else if c == b'"' {
            quoted = true;
            clean.push(c);
        } else if bytes.get(at..at + 2) == Some(b"//") {
            while at < bytes.len() && bytes[at] != b'\n' {
                at += 1;
            }
            continue;
        } else if bytes.get(at..at + 2) == Some(b"/*") {
            at += 2;
            while at + 1 < bytes.len() && &bytes[at..at + 2] != b"*/" {
                at += 1;
            }
            at += 2;
            clean.push(b' ');
            continue;
        } else {
            clean.push(c);
        }
        at += 1;
    }
    let mut out = Vec::with_capacity(clean.len());
    quoted = false;
    escaped = false;
    for (at, &c) in clean.iter().enumerate() {
        if !quoted
            && c == b','
            && clean[at + 1..]
                .iter()
                .find(|b| !b.is_ascii_whitespace())
                .is_some_and(|b| matches!(b, b']' | b'}'))
        {
            continue;
        }
        out.push(c);
        if quoted {
            if escaped {
                escaped = false;
            } else if c == b'\\' {
                escaped = true;
            } else if c == b'"' {
                quoted = false;
            }
        } else if c == b'"' {
            quoted = true;
        }
    }
    String::from_utf8(out).unwrap_or_default()
}

fn boolean(value: &Value) -> Option<bool> {
    value
        .as_bool()
        .or_else(|| match value.as_str()?.to_ascii_lowercase().as_str() {
            "yes" | "on" | "y" | "true" => Some(true),
            "no" | "off" | "n" | "false" => Some(false),
            _ => None,
        })
        .or_else(|| match value.as_i64()? {
            1 => Some(true),
            0 => Some(false),
            _ => None,
        })
}

fn frontmatter(header: &str) -> Option<Value> {
    let metadata = serde_yaml::from_str::<Value>(header).ok().or_else(|| {
        // Claude accepts unquoted scalars containing ": ". KSG repairs only plain top-level
        // scalars, not YAML collections, block strings, tags or quoted strings.
        let repaired = header
            .lines()
            .map(|line| {
                let Some((key, value)) = line.split_once(':') else {
                    return line.to_owned();
                };
                if key.is_empty()
                    || !key
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
                {
                    return line.to_owned();
                }
                let value = value.trim_start();
                let comment = value
                    .char_indices()
                    .find_map(|(i, ch)| {
                        (ch == '#' && i > 0 && value[..i].ends_with([' ', '\t'])).then_some(i)
                    })
                    .unwrap_or(value.len());
                let scalar = value[..comment].trim_end();
                if !scalar.contains(": ")
                    || scalar.starts_with([
                        '"', '\'', '[', ']', '{', '}', '|', '>', '&', '*', '!', '#', '%', '@', '`',
                    ])
                    || ["-", "?", ":"]
                        .iter()
                        .any(|p| scalar == *p || scalar.starts_with(&format!("{p} ")))
                {
                    return line.to_owned();
                }
                format!(
                    "{key}: {} {}",
                    serde_json::to_string(scalar).unwrap(),
                    &value[comment..]
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        serde_yaml::from_str::<Value>(&repaired).ok()
    })?;
    metadata.is_object().then_some(metadata)
}

fn add_dirs(args: &[String], cwd: &Path) -> Vec<PathBuf> {
    let mut directories = Vec::new();
    let mut taking = false;
    for arg in args {
        if arg == "--add-dir" {
            taking = true;
            continue;
        }
        if let Some(dir) = arg.strip_prefix("--add-dir=") {
            directories.push(cwd.join(dir));
            taking = false;
        } else if arg.starts_with('-') {
            taking = false;
        } else if taking {
            directories.push(cwd.join(arg));
        }
    }
    directories.truncate(32);
    directories
}

struct Root {
    path: PathBuf,
    prefix: Option<String>,
}

fn repository(path: &Path) -> Option<PathBuf> {
    let root = path.ancestors().find(|p| p.join(".git").exists())?;
    let marker = root.join(".git");
    if marker.is_file()
        && let Some(gitdir) =
            read(&marker).and_then(|s| s.strip_prefix("gitdir:").map(|s| root.join(s.trim())))
        && let (Some(common), Some(backlink)) = (
            read(&gitdir.join("commondir")),
            read(&gitdir.join("gitdir")),
        )
    {
        let common = gitdir.join(common.trim()).canonicalize().ok()?;
        let backlink = gitdir.join(backlink.trim()).canonicalize().ok()?;
        if backlink == marker.canonicalize().ok()?
            && gitdir.canonicalize().ok()?.parent()? == common.join("worktrees")
            && common.file_name()? == ".git"
        {
            return common.parent().map(Path::to_owned);
        }
    }
    root.canonicalize().ok()
}

fn plugin_roots(install: &Path, fallback: &str) -> Vec<Root> {
    let manifest = json_file(&install.join(".claude-plugin/plugin.json"));
    let prefix = manifest["name"].as_str().unwrap_or(fallback).to_owned();
    if !valid_name(&prefix) {
        return vec![];
    }
    let mut paths = vec![];
    if install.join("skills").is_dir() {
        paths.push(install.join("skills"));
    }
    let declared = &manifest["skills"];
    let extra: Vec<_> = declared.as_str().map(|p| vec![p]).unwrap_or_else(|| {
        declared
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .take(32)
            .collect()
    });
    for path in extra {
        let path = install.join(path);
        if let (Ok(base), Ok(real)) = (install.canonicalize(), path.canonicalize())
            && real.starts_with(base)
            && !paths.contains(&real)
        {
            paths.push(real);
        }
    }
    if paths.is_empty() && declared.is_null() && install.join("SKILL.md").is_file() {
        paths.push(install.to_owned());
    }
    paths
        .into_iter()
        .map(|path| Root {
            path,
            prefix: Some(prefix.clone()),
        })
        .collect()
}

/// No credentials or account files are read. The caller runs this on a harness worker.
pub fn claude_skills(config: &Path, cwd: &Path, args: &[String], plugins: &Path) -> Vec<Skill> {
    let mut roots = vec![
        Root {
            path: config.join("skills"),
            prefix: None,
        },
        Root {
            path: cwd.join(".claude/skills"),
            prefix: None,
        },
    ];
    roots.extend(add_dirs(args, cwd).into_iter().map(|path| Root {
        path: path.join(".claude/skills"),
        prefix: None,
    }));
    let mut overrides = BTreeMap::<String, String>::new();
    let mut enabled = BTreeMap::new();
    #[cfg(not(test))]
    let managed = if cfg!(target_os = "macos") {
        PathBuf::from("/Library/Application Support/ClaudeCode/managed-settings.json")
    } else if cfg!(windows) {
        PathBuf::from(std::env::var_os("PROGRAMDATA").unwrap_or_default())
            .join("ClaudeCode/managed-settings.json")
    } else {
        PathBuf::from("/etc/claude-code/managed-settings.json")
    };
    #[cfg(test)]
    let managed = config.join("managed-settings.json");
    let mut settings = vec![
        config.join("settings.json"),
        cwd.join(".claude/settings.json"),
        cwd.join(".claude/settings.local.json"),
    ];
    // Claude also reads a repository's local settings when launched in a child directory.
    if let Some(repo) = cwd.ancestors().find(|p| p.join(".git").exists())
        && repo != cwd
    {
        settings.push(repo.join(".claude/settings.local.json"));
    }
    settings.push(managed);
    let mut synced_hidden = false;
    for path in settings {
        let settings = json_file(&path);
        if let Some(map) = settings["skillOverrides"].as_object()
            && map.values().all(|v| {
                matches!(
                    v.as_str(),
                    Some("on" | "off" | "name-only" | "user-invocable-only")
                )
            })
        {
            overrides.extend(
                map.iter()
                    .map(|(key, value)| (key.clone(), value.as_str().unwrap().to_owned())),
            );
        }
        for (key, value) in settings["enabledPlugins"].as_object().into_iter().flatten() {
            enabled.insert(key.clone(), value.as_bool().unwrap_or(false));
        }
        if path != cwd.join(".claude/settings.json") && settings["syncClaudeAiPlugins"] == false {
            synced_hidden = true;
        }
    }
    let installed = json_file(&plugins.join("installed_plugins.json"));
    let mut claimed = std::collections::HashSet::new();
    let repository_root = repository(cwd);
    for (key, records) in installed["plugins"]
        .as_object()
        .into_iter()
        .flatten()
        .take(LIMIT)
    {
        if enabled.get(key) != Some(&true) {
            continue;
        }
        let record = records.as_array().into_iter().flatten().find(|record| {
            let scope = record["scope"].as_str().unwrap_or("");
            let applicable = matches!(scope, "user" | "managed")
                || record["projectPath"].as_str().is_some_and(|p| {
                    let path = cwd.join(p);
                    path == cwd
                        || repository_root
                            .as_ref()
                            .is_some_and(|root| repository(&path).as_ref() == Some(root))
                });
            applicable
                && record["installPath"]
                    .as_str()
                    .is_some_and(|p| cwd.join(p).is_dir())
        });
        let Some(record) = record else {
            continue;
        };
        let install = cwd.join(record["installPath"].as_str().unwrap());
        let fallback = key.split('@').next().unwrap_or(key);
        let found = plugin_roots(&install, fallback);
        for root in &found {
            if let Some(prefix) = &root.prefix {
                claimed.insert(prefix.to_lowercase());
            }
        }
        roots.extend(found);
    }
    if !synced_hidden {
        // KSG selects a synced bucket from account metadata. Shina never reads that file.
        // One marked bucket is unambiguous; with multiple accounts leave synced skills out.
        let synced = config.join("plugins/synced");
        let buckets: Vec<_> = std::fs::read_dir(&synced)
            .into_iter()
            .flatten()
            .flatten()
            .take(256)
            .filter_map(|entry| {
                entry
                    .file_name()
                    .to_str()?
                    .strip_prefix(".bucket-")
                    .map(str::to_owned)
            })
            .collect();
        if let [bucket] = buckets.as_slice()
            && !bucket.is_empty()
            && bucket.len() <= 128
            && bucket
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        {
            let bucket = synced.join(bucket);
            let manifest = json_file(&bucket.join("manifest.json"));
            for row in manifest["plugins"]
                .as_array()
                .into_iter()
                .flatten()
                .take(LIMIT)
            {
                let Some(name) = row["name"].as_str() else {
                    continue;
                };
                let directory: String = name
                    .chars()
                    .map(|c| if "<>:\"|?*\\/".contains(c) { '_' } else { c })
                    .collect();
                let directory = directory.trim_end_matches(['.', ' ']);
                if directory.is_empty()
                    || directory.starts_with('.')
                    || enabled.get(&format!("{name}@synced")) == Some(&false)
                {
                    continue;
                }
                let directory =
                    if let Some(generation) = row["generation"].as_u64().filter(|g| *g >= 2) {
                        format!("{directory}~g{generation}")
                    } else {
                        directory.into()
                    };
                let found = plugin_roots(&bucket.join(directory), name);
                let prefix = found.first().and_then(|r| r.prefix.as_ref());
                if prefix.is_some_and(|p| !claimed.insert(p.to_lowercase())) {
                    continue;
                }
                roots.extend(found);
            }
        }
    }
    let mut skills = BTreeMap::new();
    let started = std::time::Instant::now();
    let mut inspected = 0;
    for root in roots.into_iter().take(LIMIT) {
        let single = root.prefix.is_some() && root.path.join("SKILL.md").is_file();
        let mut entries: Vec<_> = if single {
            vec![root.path.clone()]
        } else {
            std::fs::read_dir(&root.path)
                .into_iter()
                .flatten()
                .flatten()
                .take(LIMIT)
                .map(|e| e.path())
                .collect()
        };
        entries.sort();
        for directory in entries {
            inspected += 1;
            if skills.len() >= LIMIT || inspected > 4096 || started.elapsed().as_secs() >= 5 {
                break;
            }
            let path = directory.join("SKILL.md");
            let Some(contents) = read(&path) else {
                continue;
            };
            let metadata = if let Some(header) = contents
                .strip_prefix("---\n")
                .or_else(|| contents.strip_prefix("---\r\n"))
            {
                let Some((header, _)) = header.split_once("\n---") else {
                    continue;
                };
                let Some(metadata) = frontmatter(header) else {
                    continue;
                };
                metadata
            } else {
                Value::Null
            };
            let folder = directory.file_name().unwrap_or_default().to_string_lossy();
            let name = if let Some(prefix) = &root.prefix {
                let raw = if single {
                    metadata["name"].as_str().unwrap_or(&folder)
                } else {
                    &folder
                };
                let raw = raw.strip_prefix(&format!("{prefix}:")).unwrap_or(raw);
                let raw: String = raw
                    .chars()
                    .map(|c| {
                        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                            c
                        } else {
                            '-'
                        }
                    })
                    .collect();
                format!("{prefix}:{raw}")
            } else {
                folder.into_owned()
            };
            if !valid_name(&name)
                || boolean(&metadata["user-invocable"]) == Some(false)
                || root.prefix.is_none() && overrides.get(&name).is_some_and(|v| v == "off")
            {
                continue;
            }
            let user_only = boolean(&metadata["disable-model-invocation"]) == Some(true)
                || root.prefix.is_none()
                    && overrides
                        .get(&name)
                        .is_some_and(|v| v == "user-invocable-only");
            skills.entry(name.clone()).or_insert(Skill {
                name,
                description: metadata["description"]
                    .as_str()
                    .unwrap_or("")
                    .chars()
                    .take(1000)
                    .collect(),
                path: Some(path),
                user_only,
            });
        }
    }
    skills.into_values().collect()
}

pub fn claude_config(inherit_custom: bool) -> Option<PathBuf> {
    #[cfg(test)]
    {
        let _ = inherit_custom;
        Some(crate::store::directory()?.join("claude-fixture"))
    }
    #[cfg(not(test))]
    {
        inherit_custom
            .then(|| std::env::var_os("CLAUDE_CONFIG_DIR"))
            .flatten()
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                    .map(|h| PathBuf::from(h).join(".claude"))
            })
    }
}

pub fn directory_path(path: PathBuf, cwd: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(relative) = text.strip_prefix("~/").or_else(|| text.strip_prefix("~\\"))
        && let Some(home) = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
    {
        return PathBuf::from(home).join(relative);
    }
    cwd.join(path)
}

pub fn command_values(skills: &[Skill]) -> Vec<Value> {
    skills
        .iter()
        .map(|s| {
            json!({"name":format!("${}", s.name),"description":s.description,
        "path":s.path,"userInvocationOnly":s.user_only,"source":"Claude"})
        })
        .collect()
}

/// Only the final native Claude slash command expands. Earlier user-only invocations include
/// KSG's explicit file fallback, and the final command gets its own last text block.
pub fn claude_dispatch(text: &str, skills: &[Skill]) -> Vec<String> {
    let mut mentions = Vec::new();
    let mut cursor = 0;
    for word in text.split_inclusive(char::is_whitespace) {
        let token = word.trim_end();
        if let Some(skill) = token
            .strip_prefix('$')
            .and_then(|name| skills.iter().find(|s| s.name == name))
        {
            mentions.push((cursor, cursor + token.len(), skill));
        }
        cursor += word.len();
    }
    let Some(&(last_start, last_end, last)) = mentions.last() else {
        return vec![text.into()];
    };
    let mut rewritten = String::new();
    let mut annotated = std::collections::HashSet::from([last.name.as_str()]);
    cursor = 0;
    for &(start, end, skill) in &mentions {
        rewritten.push_str(&text[cursor..start]);
        rewritten.push('/');
        rewritten.push_str(&skill.name);
        if skill.user_only
            && annotated.insert(skill.name.as_str())
            && let Some(path) = &skill.path
        {
            rewritten.push_str(&format!(
                " [The user invoked this skill. Follow the instructions at: {}]",
                path.display()
            ));
        }
        cursor = end;
    }
    rewritten.push_str(&text[cursor..]);
    let command = format!("/{}{}", last.name, &text[last_end..])
        .trim_end()
        .to_owned();
    if text[..last_start].trim().is_empty() {
        vec![command]
    } else {
        vec![rewritten.trim_end().into(), command]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn pi_acp_and_codex_skill_inventories_keep_only_invocable_skills() {
        let skills = from_commands(&[
            json!({"name":"skill:review","source":"skill","sourceInfo":{"path":"/skills/review/SKILL.md"},"description":"Review"}),
            json!({"name":"$acp-skill","source":"ACP"}),
            json!({"name":"$disabled","enabled":false}),
            json!({"name":"ordinary-command","source":"extension"}),
        ]);
        assert_eq!(
            skills.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            ["acp-skill", "review"]
        );
        assert_eq!(
            skills[1].path.as_deref(),
            Some(Path::new("/skills/review/SKILL.md"))
        );
        let response = json!({"data":[{"cwd":"/other","skills":[{"name":"not-here","enabled":true}]},
            {"cwd":"/project","skills":[{"name":"review","enabled":true,"path":"/a","interface":{"shortDescription":"Short"}},
            {"name":"disabled","enabled":false}]}]});
        let commands = codex_commands(&response, Path::new("/project"));
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0]["name"], "$review");
        assert_eq!(commands[0]["description"], "Short");
    }

    #[test]
    fn claude_discovery_ports_roots_precedence_plugins_and_settings_without_account_files() {
        let root = crate::store::directory().unwrap().join("skills");
        let config = root.join("config");
        let cwd = root.join("project");
        let extra = root.join("extra");
        let install = root.join("plugin");
        let plugins = config.join("plugins");
        write(
            &config.join("skills/shared/SKILL.md"),
            "---\ndescription: user copy\n---\nbody",
        );
        write(
            &cwd.join(".claude/skills/shared/SKILL.md"),
            "---\ndescription: project copy\n---\nbody",
        );
        write(
            &cwd.join(".claude/skills/hidden/SKILL.md"),
            "---\nuser-invocable: no\n---\nbody",
        );
        write(&config.join("skills/off/SKILL.md"), "body");
        write(
            &extra.join(".claude/skills/extra/SKILL.md"),
            "---\ndisable-model-invocation: yes\n---\nbody",
        );
        write(
            &install.join(".claude-plugin/plugin.json"),
            "{\"name\":\"plugin\",\"skills\":[\"custom\"]}",
        );
        write(
            &install.join("custom/task/SKILL.md"),
            "---\ndescription: plugin task: with a colon\n---\nbody",
        );
        write(
            &plugins.join("installed_plugins.json"),
            &json!({"plugins":{"plugin@test":[{"scope":"user","installPath":install}]}})
                .to_string(),
        );
        write(
            &config.join("settings.json"),
            "{ // a comment\n\"enabledPlugins\":{\"plugin@test\":true,},\"skillOverrides\":{\"off\":\"off\",},}",
        );
        let args = vec!["--add-dir".into(), extra.to_string_lossy().into_owned()];
        let skills = claude_skills(&config, &cwd, &args, &plugins);
        assert_eq!(
            skills.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            ["extra", "plugin:task", "shared"]
        );
        assert_eq!(skills[2].description, "user copy");
        assert!(skills[0].user_only);
        assert_eq!(skills[1].description, "plugin task: with a colon");
        assert!(!config.join(".claude.json").exists());
    }

    #[test]
    fn claude_synced_plugins_require_an_unambiguous_marked_bucket() {
        let root = crate::store::directory().unwrap().join("synced");
        let config = root.join("config");
        let cwd = root.join("project");
        std::fs::create_dir_all(&cwd).unwrap();
        let synced = config.join("plugins/synced");
        write(&synced.join(".bucket-fixture"), "");
        write(
            &synced.join("fixture/manifest.json"),
            r#"{"plugins":[{"name":"sample","generation":2}]}"#,
        );
        write(
            &synced.join("fixture/sample~g2/skills/task/SKILL.md"),
            "body",
        );
        assert_eq!(
            claude_skills(&config, &cwd, &[], &config.join("plugins"))[0].name,
            "sample:task"
        );
        write(&synced.join(".bucket-other-account"), "");
        assert!(claude_skills(&config, &cwd, &[], &config.join("plugins")).is_empty());
    }

    #[test]
    fn claude_dispatch_matches_ksg_native_command_blocks() {
        let skills = vec![
            Skill {
                name: "first".into(),
                description: String::new(),
                path: Some("/skills/first/SKILL.md".into()),
                user_only: true,
            },
            Skill {
                name: "second".into(),
                description: String::new(),
                path: None,
                user_only: false,
            },
        ];
        assert_eq!(
            claude_dispatch("$second do this", &skills),
            ["/second do this"]
        );
        assert_eq!(
            claude_dispatch("please $second do this", &skills),
            ["please /second do this", "/second do this"]
        );
        assert_eq!(
            claude_dispatch("$first and $second", &skills),
            [
                "/first [The user invoked this skill. Follow the instructions at: /skills/first/SKILL.md] and /second",
                "/second"
            ]
        );
        assert_eq!(
            claude_dispatch("$first and $first", &skills),
            ["/first and /first", "/first"]
        );
        assert_eq!(
            claude_dispatch("keep $unknown and $HOME", &skills),
            ["keep $unknown and $HOME"]
        );
    }
}
