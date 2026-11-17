//! Stable logical identities layered over Shina's existing server-local records.
//!
//! Numeric Captain/thread/project/routine keys and string task ids remain the
//! canonical storage references. This module maps those aliases to UUIDs in a
//! sidecar catalog, so older receipts, provider sessions and projection rows
//! continue to address the same records byte for byte.
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

pub const INDEX_VERSION: u64 = 5;
pub const SIDECAR_SCHEMA: u32 = 1;
pub const CAPTAIN: &str = "captain";
pub const THREAD: &str = "thread";
pub const PROJECT_CHECKOUT: &str = "project_checkout";
pub const TASK: &str = "task";
pub const ROUTINE: &str = "routine";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Migration {
    pub backup_path: String,
    pub source_index_sha256: String,
    pub backup_sha256: String,
    pub migrated_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditSubject {
    pub subject: String,
    pub kind: String,
    pub legacy_key: String,
    pub logical_id: String,
    pub workspace_id: String,
    pub owner_id: String,
    pub controller_id: String,
}

/// Additive identities already persisted in an index or the SQLite sidecar.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    pub schema: u32,
    pub workspace_id: String,
    pub owner_id: String,
    pub controller_id: String,
    pub executor_id: String,
    #[serde(default)]
    pub captain_ids: BTreeMap<String, String>,
    #[serde(default)]
    pub thread_ids: BTreeMap<String, String>,
    #[serde(default)]
    pub project_checkout_ids: BTreeMap<String, String>,
    #[serde(default)]
    pub task_ids: BTreeMap<String, String>,
    #[serde(default)]
    pub routine_ids: BTreeMap<String, String>,
    #[serde(default)]
    pub audit_subjects: BTreeMap<String, AuditSubject>,
    #[serde(default)]
    pub migration: Option<Migration>,
}

impl Catalog {
    /// The no-I/O identity view used by an in-memory or read-only snapshot.
    pub fn local(directory: &Path) -> Self {
        let seed = crate::projects::clean_path(directory);
        let seed = seed.to_string_lossy();
        let controller = controller_from_disk(directory)
            .unwrap_or_else(|| deterministic_uuid("shina.identity.controller.v1", [seed.as_ref()]));
        let workspace = deterministic_uuid("shina.identity.workspace.v1", [seed.as_ref()]);
        Self::with_workspace(&controller, &workspace)
    }

    pub fn new(controller_id: &str) -> Self {
        let workspace_id = deterministic_uuid("shina.identity.workspace.v1", [controller_id]);
        Self::with_workspace(controller_id, &workspace_id)
    }

    pub fn with_workspace(controller_id: &str, workspace_id: &str) -> Self {
        let owner_id = deterministic_uuid("shina.identity.owner.v1", [workspace_id]);
        Self {
            schema: SIDECAR_SCHEMA,
            workspace_id: workspace_id.into(),
            owner_id,
            controller_id: controller_id.into(),
            executor_id: controller_id.into(),
            captain_ids: BTreeMap::new(),
            thread_ids: BTreeMap::new(),
            project_checkout_ids: BTreeMap::new(),
            task_ids: BTreeMap::new(),
            routine_ids: BTreeMap::new(),
            audit_subjects: BTreeMap::new(),
            migration: None,
        }
    }

    pub fn from_index(index: &Value) -> Result<Option<Self>, String> {
        let Some(value) = index.get("identity") else {
            if index["version"] == INDEX_VERSION {
                return Err(
                    "Identity index is missing its catalog. Restore the matched backup.".into(),
                );
            }
            return Ok(None);
        };
        let catalog: Self = serde_json::from_value(value.clone())
            .map_err(|e| format!("Invalid identity catalog: {e}"))?;
        catalog.validate()?;
        if index["_partial"] != true {
            catalog.validate_record_aliases(index)?;
        }
        Ok(Some(catalog))
    }

    fn validate_record_aliases(&self, index: &Value) -> Result<(), String> {
        let captain_count = index["captains"]
            .as_array()
            .ok_or("The saved captain profiles are invalid.")?
            .len();
        for captain in 0..captain_count {
            if self.captain_uid(captain).is_none() {
                return Err(format!("Captain {captain} has no stable logical identity."));
            }
        }
        for key in self.captain_ids.keys() {
            let captain = key
                .parse::<usize>()
                .map_err(|_| "Invalid Captain alias in the identity catalog.".to_owned())?;
            if captain >= captain_count {
                return Err(format!(
                    "Captain alias {captain} has no retained roster slot. Restore the matched backup."
                ));
            }
        }
        let projects = index["projects"]
            .as_array()
            .ok_or("Invalid project list.")?;
        let canonical_projects = canonical_project_aliases(projects)?;
        for project in projects {
            let id = project["id"]
                .as_u64()
                .and_then(|id| usize::try_from(id).ok())
                .ok_or("Invalid project id.")?;
            if self.checkout_uid(id).is_none() {
                return Err(format!("Project {id} has no stable checkout identity."));
            }
            let canonical = canonical_projects
                .get(&id.to_string())
                .ok_or("Project alias was not canonicalized.")?;
            if self.checkout_uid(id)
                != self.checkout_uid(
                    canonical
                        .parse()
                        .map_err(|_| "Invalid canonical project id.".to_owned())?,
                )
            {
                return Err(format!("Project {id} has a checkout identity collision."));
            }
        }
        for field in ["threads", "threadViews"] {
            for thread in index[field].as_array().into_iter().flatten() {
                let Some(id) = thread["id"]
                    .as_u64()
                    .and_then(|id| usize::try_from(id).ok())
                else {
                    continue;
                };
                if id != 0 && id != usize::MAX && self.thread_uid(id).is_none() {
                    return Err(format!("Thread {id} has no stable logical identity."));
                }
            }
        }
        for task in index["tasks"].as_array().into_iter().flatten() {
            let id = task["id"].as_str().ok_or("Invalid task id.")?;
            if self.task_uid(id).is_none() {
                return Err(format!("Task {id} has no stable logical identity."));
            }
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema != SIDECAR_SCHEMA {
            return Err("Identity catalog requires a newer Shina binary.".into());
        }
        for (name, value) in [
            ("workspaceId", self.workspace_id.as_str()),
            ("ownerId", self.owner_id.as_str()),
            ("controllerId", self.controller_id.as_str()),
            ("executorId", self.executor_id.as_str()),
        ] {
            uuid::Uuid::parse_str(value).map_err(|_| format!("Invalid identity {name}."))?;
        }
        if self.workspace_id == self.owner_id {
            return Err("Workspace and owner identities must be distinct.".into());
        }
        let mut logical: BTreeMap<String, (&str, String)> = BTreeMap::new();
        for (kind, ids) in self.maps() {
            for (legacy, id) in ids {
                uuid::Uuid::parse_str(id)
                    .map_err(|_| format!("Invalid {kind} identity for {legacy}."))?;
                if let Some((previous_kind, previous)) =
                    logical.insert(id.clone(), (kind, legacy.clone()))
                    && (previous_kind != kind || kind != PROJECT_CHECKOUT)
                {
                    return Err(format!(
                        "Identity collision: {previous_kind}:{previous} and {kind}:{legacy} share {id}."
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn maps(&self) -> [(&'static str, &BTreeMap<String, String>); 5] {
        [
            (CAPTAIN, &self.captain_ids),
            (THREAD, &self.thread_ids),
            (PROJECT_CHECKOUT, &self.project_checkout_ids),
            (TASK, &self.task_ids),
            (ROUTINE, &self.routine_ids),
        ]
    }

    fn map_mut(&mut self, kind: &str) -> Option<&mut BTreeMap<String, String>> {
        match kind {
            CAPTAIN => Some(&mut self.captain_ids),
            THREAD => Some(&mut self.thread_ids),
            PROJECT_CHECKOUT => Some(&mut self.project_checkout_ids),
            TASK => Some(&mut self.task_ids),
            ROUTINE => Some(&mut self.routine_ids),
            _ => None,
        }
    }

    pub fn add_alias(&mut self, kind: &str, legacy_key: impl Into<String>, canonical_key: &str) {
        let legacy_key = legacy_key.into();
        let id = deterministic_uuid(
            "shina.identity.entity.v1",
            [
                self.workspace_id.as_str(),
                self.controller_id.as_str(),
                kind,
                canonical_key,
            ],
        );
        let map = self
            .map_mut(kind)
            .unwrap_or_else(|| panic!("unsupported identity kind {kind}"));
        map.entry(legacy_key).or_insert(id);
    }

    pub fn captain_uid(&self, captain: usize) -> Option<&str> {
        self.captain_ids
            .get(&captain.to_string())
            .map(String::as_str)
    }

    pub fn thread_uid(&self, thread: usize) -> Option<&str> {
        self.thread_ids.get(&thread.to_string()).map(String::as_str)
    }

    pub fn checkout_uid(&self, project: usize) -> Option<&str> {
        self.project_checkout_ids
            .get(&project.to_string())
            .map(String::as_str)
    }

    pub fn task_uid(&self, task: &str) -> Option<&str> {
        self.task_ids.get(task).map(String::as_str)
    }

    pub fn routine_uid(&self, routine: u64) -> Option<&str> {
        self.routine_ids
            .get(&routine.to_string())
            .map(String::as_str)
    }

    /// Add current in-memory records without changing any legacy key.
    pub fn with_current(
        &self,
        captains: impl IntoIterator<Item = usize>,
        projects: impl IntoIterator<Item = usize>,
        threads: impl IntoIterator<Item = usize>,
        tasks: impl IntoIterator<Item = String>,
        routines: impl IntoIterator<Item = u64>,
    ) -> Self {
        let mut catalog = self.clone();
        for captain in captains {
            let key = captain.to_string();
            catalog.add_alias(CAPTAIN, key.clone(), &key);
        }
        for project in projects {
            let key = project.to_string();
            catalog.add_alias(PROJECT_CHECKOUT, key.clone(), &key);
        }
        for thread in threads {
            let key = thread.to_string();
            catalog.add_alias(THREAD, key.clone(), &key);
        }
        for task in tasks {
            catalog.add_alias(TASK, task.clone(), &task);
        }
        for routine in routines {
            let key = routine.to_string();
            catalog.add_alias(ROUTINE, key.clone(), &key);
        }
        catalog
    }

    /// Add the catalog and per-record canonical fields to an authoritative read.
    pub fn annotate_index(&self, index: &mut Value) {
        index["version"] = json!(INDEX_VERSION);
        index["identity"] = serde_json::to_value(self).expect("identity catalog serialization");
        if let Some(records) = index.get_mut("captains").and_then(Value::as_array_mut) {
            for (offset, record) in records.iter_mut().enumerate() {
                if let Some(uid) = self.captain_uid(offset) {
                    record["captainUid"] = json!(uid);
                }
            }
        }
        if let Some(records) = index.get_mut("projects").and_then(Value::as_array_mut) {
            for record in records {
                if let Some(uid) = record["id"]
                    .as_u64()
                    .and_then(|id| usize::try_from(id).ok())
                    .and_then(|id| self.checkout_uid(id))
                {
                    record["checkoutUid"] = json!(uid);
                }
            }
        }
        for field in ["threads", "threadViews"] {
            if let Some(records) = index.get_mut(field).and_then(Value::as_array_mut) {
                for record in records {
                    if let Some(uid) = record["id"]
                        .as_u64()
                        .and_then(|id| usize::try_from(id).ok())
                        .and_then(|id| self.thread_uid(id))
                    {
                        record["threadUid"] = json!(uid);
                    }
                }
            }
        }
        if let Some(records) = index.get_mut("tasks").and_then(Value::as_array_mut) {
            for record in records {
                if let Some(uid) = record["id"].as_str().and_then(|id| self.task_uid(id)) {
                    record["taskUid"] = json!(uid);
                }
            }
        }
        for field in ["captainReports", "captainQuestionAnswers"] {
            if let Some(records) = index.get_mut(field).and_then(Value::as_array_mut) {
                for record in records {
                    if let Some(uid) = record["captain"]
                        .as_u64()
                        .and_then(|id| usize::try_from(id).ok())
                        .and_then(|id| self.captain_uid(id))
                    {
                        record["captainUid"] = json!(uid);
                    }
                    if let Some(uid) = record["threadId"]
                        .as_u64()
                        .and_then(|id| usize::try_from(id).ok())
                        .and_then(|id| self.thread_uid(id))
                    {
                        record["threadUid"] = json!(uid);
                    }
                }
            }
        }
    }

    pub fn audit_subjects_from(
        &mut self,
        subjects: impl IntoIterator<Item = String>,
    ) -> Result<(), String> {
        for subject in subjects {
            if !subject.starts_with("mcp/") {
                continue;
            }
            let (kind, legacy_key, logical_id) =
                match subject.as_str() {
                    value if value.starts_with("mcp/captain/") => {
                        let key = value.trim_start_matches("mcp/captain/");
                        let id = self.captain_ids.get(key).ok_or_else(|| {
                            format!("Audit subject {subject} has no Captain alias.")
                        })?;
                        (CAPTAIN, key.to_owned(), id.clone())
                    }
                    value if value.starts_with("mcp/delegate/captain/") => {
                        let key = value.trim_start_matches("mcp/delegate/captain/");
                        let id = self.captain_ids.get(key).ok_or_else(|| {
                            format!("Audit subject {subject} has no Captain alias.")
                        })?;
                        (CAPTAIN, key.to_owned(), id.clone())
                    }
                    value if value.starts_with("mcp/delegate/thread/") => {
                        let key = value.trim_start_matches("mcp/delegate/thread/");
                        let id = self.thread_ids.get(key).cloned().unwrap_or_else(|| {
                            // A deleted child thread may no longer have an index record,
                            // but its committed retry receipt remains replayable.
                            self.add_alias(THREAD, key.to_owned(), key);
                            self.thread_ids[key].clone()
                        });
                        (THREAD, key.to_owned(), id)
                    }
                    _ => continue,
                };
            self.audit_subjects.insert(
                subject.clone(),
                AuditSubject {
                    subject,
                    kind: kind.into(),
                    legacy_key,
                    logical_id,
                    workspace_id: self.workspace_id.clone(),
                    owner_id: self.owner_id.clone(),
                    controller_id: self.controller_id.clone(),
                },
            );
        }
        Ok(())
    }
}

pub fn deterministic_uuid<'a>(namespace: &str, parts: impl IntoIterator<Item = &'a str>) -> String {
    let mut digest = Sha256::new();
    digest.update(namespace.as_bytes());
    for part in parts {
        digest.update((part.len() as u64).to_be_bytes());
        digest.update(part.as_bytes());
    }
    let digest = digest.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(bytes).to_string()
}

pub fn controller_from_disk(directory: &Path) -> Option<String> {
    let text = std::fs::read_to_string(directory.join("server-id")).ok()?;
    uuid::Uuid::parse_str(text.trim())
        .ok()
        .map(|id| id.to_string())
}

/// Migrate an index version 3 or 4 to version 5 without changing legacy fields.
#[cfg(test)]
pub fn migrate_index(
    index: &mut Value,
    controller_id: &str,
    routines: Option<&Value>,
    migration: Option<Migration>,
) -> Result<Catalog, String> {
    let workspace_id = Catalog::new(controller_id).workspace_id;
    migrate_index_with_workspace(index, controller_id, &workspace_id, routines, migration)
}

pub fn migrate_index_with_workspace(
    index: &mut Value,
    controller_id: &str,
    workspace_id: &str,
    routines: Option<&Value>,
    migration: Option<Migration>,
) -> Result<Catalog, String> {
    if !index.is_object() {
        return Err("Saved index is not an object.".into());
    }
    if index["version"] != 3 && index["version"] != 4 && index["version"] != INDEX_VERSION {
        return Err("Unsupported index version.".into());
    }
    if index["version"] == INDEX_VERSION
        && let Some(catalog) = Catalog::from_index(index)?
    {
        return Ok(catalog);
    }
    let mut catalog = Catalog::with_workspace(controller_id, workspace_id);
    catalog.migration = migration;
    let captains = index["captains"]
        .as_array()
        .ok_or("The saved captain profiles are invalid.")?;
    let projects = index["projects"]
        .as_array()
        .ok_or("Invalid project list.")?;
    let threads = index["threads"].as_array().ok_or("Invalid thread list.")?;
    let tasks = index["tasks"].as_array().cloned().unwrap_or_default();

    for captain in 0..captains.len() {
        let key = captain.to_string();
        catalog.add_alias(CAPTAIN, key.clone(), &key);
    }

    let project_aliases = canonical_project_aliases(projects)?;
    for (alias, canonical) in &project_aliases {
        catalog.add_alias(PROJECT_CHECKOUT, alias.clone(), canonical);
    }
    for project in projects {
        let id = project["id"]
            .as_u64()
            .and_then(|id| usize::try_from(id).ok())
            .filter(|id| *id > 0 && *id != usize::MAX)
            .ok_or("Invalid project id.")?;
        let uid = catalog
            .checkout_uid(id)
            .ok_or("Project checkout identity was not allocated.")?;
        require_matching_uid(project, "checkoutUid", uid, "project checkout")?;
    }

    let mut thread_ids = BTreeSet::new();
    for record in threads {
        let id = record["id"]
            .as_u64()
            .and_then(|id| usize::try_from(id).ok())
            .filter(|id| *id != 0 && *id != usize::MAX)
            .ok_or("Invalid thread id.")?;
        if !thread_ids.insert(id) {
            return Err("Duplicate thread ids.".into());
        }
        let key = id.to_string();
        catalog.add_alias(THREAD, key.clone(), &key);
        let uid = catalog
            .thread_uid(id)
            .ok_or("Thread identity was not allocated.")?;
        require_matching_uid(record, "threadUid", uid, "thread")?;
    }
    if let Some(views) = index.get("threadViews").and_then(Value::as_array) {
        for record in views {
            let Some(id) = record["id"]
                .as_u64()
                .and_then(|id| usize::try_from(id).ok())
                .filter(|id| *id != 0 && *id != usize::MAX)
            else {
                continue;
            };
            if !thread_ids.contains(&id) {
                return Err(format!("Thread view {id} has no matching thread."));
            }
            let uid = catalog
                .thread_uid(id)
                .ok_or("Thread identity was not allocated.")?;
            require_matching_uid(record, "threadUid", uid, "thread view")?;
        }
    }

    let mut task_ids = BTreeSet::new();
    for task in &tasks {
        let id = task["id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or("Invalid task id.")?;
        if !task_ids.insert(id.to_owned()) {
            return Err(format!("Duplicate task id {id}."));
        }
        catalog.add_alias(TASK, id.to_owned(), id);
        let uid = catalog
            .task_uid(id)
            .ok_or("Task identity was not allocated.")?;
        require_matching_uid(task, "taskUid", uid, "task")?;
    }

    if let Some(routines) = routines {
        let entries = routines["routines"]
            .as_array()
            .ok_or("Invalid routine list during identity migration.")?;
        let mut routine_ids = BTreeSet::new();
        for routine in entries {
            let id = routine["id"]
                .as_u64()
                .filter(|id| *id > 0)
                .ok_or("Invalid routine id during identity migration.")?;
            if !routine_ids.insert(id) {
                return Err(format!("Duplicate routine id {id}."));
            }
            let key = id.to_string();
            catalog.add_alias(ROUTINE, key.clone(), &key);
            let uid = catalog
                .routine_uid(id)
                .ok_or("Routine identity was not allocated.")?;
            require_matching_uid(routine, "routineUid", uid, "routine")?;
        }
    }

    validate_references(index, routines, &catalog, &project_aliases)?;
    catalog.annotate_index(index);
    Ok(catalog)
}

/// Build a catalog for a legacy in-memory restore. Runtime validation still
/// runs through the existing record loader; persisted migrations use the
/// fail-closed checks above.
pub fn catalog_from_legacy(index: &Value, controller_id: &str) -> Result<Catalog, String> {
    let mut catalog = Catalog::new(controller_id);
    let captains = index["captains"]
        .as_array()
        .ok_or("The saved captain profiles are invalid.")?;
    for captain in 0..captains.len() {
        let key = captain.to_string();
        catalog.add_alias(CAPTAIN, key.clone(), &key);
    }
    let projects = index["projects"]
        .as_array()
        .ok_or("Invalid project list.")?;
    for (alias, canonical) in canonical_project_aliases(projects)? {
        catalog.add_alias(PROJECT_CHECKOUT, alias, &canonical);
    }
    for thread in index["threads"].as_array().into_iter().flatten() {
        let Some(id) = thread["id"]
            .as_u64()
            .and_then(|id| usize::try_from(id).ok())
            .filter(|id| *id != 0 && *id != usize::MAX)
        else {
            continue;
        };
        let key = id.to_string();
        catalog.add_alias(THREAD, key.clone(), &key);
    }
    for task in index["tasks"].as_array().into_iter().flatten() {
        if let Some(id) = task["id"].as_str() {
            catalog.add_alias(TASK, id.to_owned(), id);
        }
    }
    Ok(catalog)
}

fn require_matching_uid(
    record: &Value,
    field: &str,
    uid: &str,
    description: &str,
) -> Result<(), String> {
    if let Some(existing) = record.get(field).and_then(Value::as_str)
        && existing != uid
    {
        return Err(format!(
            "The {description} has a conflicting canonical identity in {field}."
        ));
    }
    Ok(())
}

fn canonical_project_aliases(projects: &[Value]) -> Result<BTreeMap<String, String>, String> {
    let mut seen_ids = BTreeSet::new();
    let mut canonical: Vec<((String, String), String)> = Vec::new();
    let mut aliases = BTreeMap::new();
    for project in projects {
        let id = project["id"]
            .as_u64()
            .and_then(|id| usize::try_from(id).ok())
            .filter(|id| *id > 0 && *id != usize::MAX)
            .ok_or("Invalid project id.")?;
        if !seen_ids.insert(id) {
            return Err("Duplicate project ids.".into());
        }
        let machine = project["machine"]
            .as_str()
            .ok_or("Invalid project machine.")?;
        let path = project["path"].as_str().ok_or("Invalid project path.")?;
        let normalized = if machine == "local" {
            crate::projects::normalize_path(Path::new(path))
                .to_string_lossy()
                .into_owned()
        } else {
            path.to_owned()
        };
        let key = (machine.to_owned(), normalized);
        let canonical_id = canonical
            .iter()
            .find(|(candidate, _)| candidate == &key)
            .map(|(_, id)| id.clone())
            .unwrap_or_else(|| {
                let id = id.to_string();
                canonical.push((key, id.clone()));
                id
            });
        aliases.insert(id.to_string(), canonical_id);
    }
    Ok(aliases)
}

fn validate_references(
    index: &Value,
    routines: Option<&Value>,
    catalog: &Catalog,
    project_aliases: &BTreeMap<String, String>,
) -> Result<(), String> {
    let captains: BTreeSet<usize> = catalog
        .captain_ids
        .keys()
        .filter_map(|key| key.parse().ok())
        .collect();
    let threads: BTreeSet<usize> = catalog
        .thread_ids
        .keys()
        .filter_map(|key| key.parse().ok())
        .collect();
    let projects: BTreeSet<usize> = project_aliases
        .keys()
        .filter_map(|key| key.parse().ok())
        .collect();

    for project in index["projects"].as_array().into_iter().flatten() {
        require_id(&project["captain"], &captains, "Captain", "project.captain")?;
        require_id(
            &project["captainThread"],
            &threads,
            "thread",
            "project.captainThread",
        )?;
    }
    for thread in index["threads"].as_array().into_iter().flatten() {
        require_id(&thread["captain"], &captains, "Captain", "thread.captain")?;
        require_id(
            &thread["crewCaptain"],
            &captains,
            "Captain",
            "thread.crewCaptain",
        )?;
        require_id(&thread["parent"], &threads, "thread", "thread.parent")?;
        require_id(&thread["project"], &projects, "project", "thread.project")?;
    }
    for task in index["tasks"].as_array().into_iter().flatten() {
        require_id(
            field(task, "parent_thread_id", "parentThreadId"),
            &threads,
            "thread",
            "task.parentThreadId",
        )?;
        require_id(
            field(task, "child_thread_id", "childThreadId"),
            &threads,
            "thread",
            "task.childThreadId",
        )?;
        require_id(
            field(task, "project_id", "projectId"),
            &projects,
            "project",
            "task.projectId",
        )?;
        require_id(
            field(task, "coordinator_captain", "coordinatorCaptain"),
            &captains,
            "Captain",
            "task.coordinatorCaptain",
        )?;
    }
    for field_name in [
        "captainScopes",
        "captainRemoteScopes",
        "captainHomes",
        "captainBooks",
        "captainLifecycles",
        "captainReportPolicies",
    ] {
        if let Some(map) = index[field_name].as_object() {
            for key in map.keys() {
                let id = key
                    .parse::<usize>()
                    .map_err(|_| format!("Invalid Captain key in {field_name}."))?;
                if !captains.contains(&id) {
                    return Err(format!("Missing Captain {id} referenced by {field_name}."));
                }
            }
        }
    }
    if let Some(homes) = index["captainHomes"].as_object() {
        for home in homes.values() {
            require_id(home, &threads, "thread", "captainHomes")?;
        }
    }
    for report in index["captainReports"].as_array().into_iter().flatten() {
        require_id(
            &report["captain"],
            &captains,
            "Captain",
            "captainReports.captain",
        )?;
        require_id(
            &report["threadId"],
            &threads,
            "thread",
            "captainReports.threadId",
        )?;
    }
    for answer in index["captainQuestionAnswers"]
        .as_array()
        .into_iter()
        .flatten()
    {
        require_id(
            &answer["captain"],
            &captains,
            "Captain",
            "captainQuestionAnswers.captain",
        )?;
        require_id(
            &answer["threadId"],
            &threads,
            "thread",
            "captainQuestionAnswers.threadId",
        )?;
    }
    if let Some(routines) = routines {
        for routine in routines["routines"].as_array().into_iter().flatten() {
            require_id(&routine["captain"], &captains, "Captain", "routine.captain")?;
            require_id(&routine["home"], &threads, "thread", "routine.home")?;
        }
    }
    Ok(())
}

fn field<'a>(value: &'a Value, snake: &str, camel: &str) -> &'a Value {
    value
        .get(snake)
        .or_else(|| value.get(camel))
        .unwrap_or(&Value::Null)
}

fn require_id(
    value: &Value,
    known: &BTreeSet<usize>,
    description: &str,
    reference: &str,
) -> Result<(), String> {
    if value.is_null() {
        return Ok(());
    }
    let id = value
        .as_u64()
        .and_then(|id| usize::try_from(id).ok())
        .ok_or_else(|| format!("Invalid {description} reference in {reference}."))?;
    if !known.contains(&id) {
        return Err(format!(
            "Missing {description} {id} referenced by {reference}."
        ));
    }
    Ok(())
}

pub(crate) fn read_index(connection: &Connection) -> Result<Option<(Value, String)>, String> {
    if let Some(value) = crate::projection_store::read(connection, "index.json")? {
        return Ok(Some((value.clone(), value.to_string())));
    }
    let text: Option<String> = connection
        .query_row(
            "SELECT payload FROM documents WHERE name='index.json'",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    text.map(|text| {
        serde_json::from_str(&text)
            .map(|value| (value, text))
            .map_err(|e| e.to_string())
    })
    .transpose()
}

pub(crate) fn read_routines(connection: &Connection) -> Result<Option<Value>, String> {
    let text: Option<String> = connection
        .query_row(
            "SELECT payload FROM documents WHERE name='routines.json'",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    text.map(|text| serde_json::from_str(&text).map_err(|e| e.to_string()))
        .transpose()
}

pub(crate) fn write_catalog(
    connection: &Connection,
    catalog: &Catalog,
    backup_path: Option<&str>,
    source_index_sha256: Option<&str>,
    backup_sha256: Option<&str>,
) -> Result<(), String> {
    catalog.validate()?;
    connection
        .execute(
            "INSERT INTO identity_workspace(
                singleton,workspace_id,owner_id,controller_id,executor_id,identity_schema,
                backup_path,source_index_sha256,backup_sha256,migrated_at,index_version
             ) VALUES(1,?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
             ON CONFLICT(singleton) DO UPDATE SET
                workspace_id=excluded.workspace_id,owner_id=excluded.owner_id,
                controller_id=excluded.controller_id,executor_id=excluded.executor_id,
                identity_schema=excluded.identity_schema,
                backup_path=COALESCE(identity_workspace.backup_path,excluded.backup_path),
                source_index_sha256=COALESCE(identity_workspace.source_index_sha256,excluded.source_index_sha256),
                backup_sha256=COALESCE(identity_workspace.backup_sha256,excluded.backup_sha256),
                migrated_at=CASE WHEN excluded.migrated_at>0
                    THEN MIN(identity_workspace.migrated_at,excluded.migrated_at)
                    ELSE identity_workspace.migrated_at END,
                index_version=excluded.index_version",
            params![
                catalog.workspace_id,
                catalog.owner_id,
                catalog.controller_id,
                catalog.executor_id,
                catalog.schema,
                backup_path,
                source_index_sha256,
                backup_sha256,
                catalog.migration.as_ref().map_or(0, |m| m.migrated_at as i64),
                INDEX_VERSION as i64
            ],
        )
        .map_err(|e| e.to_string())?;
    for (kind, ids) in catalog.maps() {
        for (legacy, logical) in ids {
            connection
                .execute(
                    "INSERT INTO identity_entities(controller_id,kind,logical_id,canonical_key)
                     VALUES(?1,?2,?3,?4)
                     ON CONFLICT DO NOTHING",
                    params![catalog.controller_id, kind, logical, legacy],
                )
                .map_err(|e| e.to_string())?;
            let stored: Option<String> = connection
                .query_row(
                    "SELECT logical_id FROM identity_aliases
                     WHERE controller_id=?1 AND kind=?2 AND legacy_key=?3",
                    params![catalog.controller_id, kind, legacy],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|e| e.to_string())?;
            if let Some(stored) = stored {
                if &stored != logical {
                    return Err(format!(
                        "Identity alias collision for {kind}:{legacy}; restore the matched backup."
                    ));
                }
            } else {
                connection
                    .execute(
                        "INSERT INTO identity_aliases(controller_id,kind,legacy_key,logical_id)
                         VALUES(?1,?2,?3,?4)",
                        params![catalog.controller_id, kind, legacy, logical],
                    )
                    .map_err(|e| e.to_string())?;
            }
        }
    }
    for subject in catalog.audit_subjects.values() {
        connection
            .execute(
                "INSERT INTO identity_audit_subjects(
                    subject,kind,legacy_key,logical_id,workspace_id,owner_id,controller_id
                 ) VALUES(?1,?2,?3,?4,?5,?6,?7)
                 ON CONFLICT(subject) DO NOTHING",
                params![
                    subject.subject,
                    subject.kind,
                    subject.legacy_key,
                    subject.logical_id,
                    subject.workspace_id,
                    subject.owner_id,
                    subject.controller_id
                ],
            )
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub(crate) fn load_catalog(connection: &Connection) -> Result<Option<Catalog>, String> {
    let workspace: Option<(
        String,
        String,
        String,
        String,
        u32,
        Option<String>,
        Option<String>,
        Option<String>,
        i64,
    )> = connection
        .query_row(
            "SELECT workspace_id,owner_id,controller_id,executor_id,identity_schema,
                    backup_path,source_index_sha256,backup_sha256,migrated_at
             FROM identity_workspace WHERE singleton=1",
            [],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                ))
            },
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let Some((
        workspace_id,
        owner_id,
        controller_id,
        executor_id,
        schema,
        backup_path,
        source_index_sha256,
        backup_sha256,
        migrated_at,
    )) = workspace
    else {
        return Ok(None);
    };
    let mut catalog = Catalog {
        schema,
        workspace_id,
        owner_id,
        controller_id: controller_id.clone(),
        executor_id,
        captain_ids: BTreeMap::new(),
        thread_ids: BTreeMap::new(),
        project_checkout_ids: BTreeMap::new(),
        task_ids: BTreeMap::new(),
        routine_ids: BTreeMap::new(),
        audit_subjects: BTreeMap::new(),
        migration: backup_path.map(|backup_path| Migration {
            backup_path,
            source_index_sha256: source_index_sha256.unwrap_or_default(),
            backup_sha256: backup_sha256.unwrap_or_default(),
            migrated_at: migrated_at.max(0) as u64,
        }),
    };
    let mut statement = connection
        .prepare(
            "SELECT kind,legacy_key,logical_id FROM identity_aliases
             WHERE controller_id=?1 ORDER BY kind,legacy_key",
        )
        .map_err(|e| e.to_string())?;
    for row in statement
        .query_map([&controller_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?
    {
        let (kind, legacy, logical) = row.map_err(|e| e.to_string())?;
        catalog
            .map_mut(&kind)
            .ok_or_else(|| format!("Unknown identity kind {kind}."))?
            .insert(legacy, logical);
    }
    let mut statement = connection
        .prepare(
            "SELECT subject,kind,legacy_key,logical_id,workspace_id,owner_id,controller_id
             FROM identity_audit_subjects WHERE controller_id=?1 ORDER BY subject",
        )
        .map_err(|e| e.to_string())?;
    for row in statement
        .query_map([&catalog.controller_id], |r| {
            Ok(AuditSubject {
                subject: r.get(0)?,
                kind: r.get(1)?,
                legacy_key: r.get(2)?,
                logical_id: r.get(3)?,
                workspace_id: r.get(4)?,
                owner_id: r.get(5)?,
                controller_id: r.get(6)?,
            })
        })
        .map_err(|e| e.to_string())?
    {
        let subject = row.map_err(|e| e.to_string())?;
        catalog
            .audit_subjects
            .insert(subject.subject.clone(), subject);
    }
    catalog.validate()?;
    Ok(Some(catalog))
}

pub(crate) fn identity_value(connection: &Connection) -> Result<Option<Value>, String> {
    load_catalog(connection)?
        .map(|catalog| serde_json::to_value(catalog).map_err(|e| e.to_string()))
        .transpose()
}

pub(crate) fn decorate_index(connection: &Connection, index: &mut Value) -> Result<(), String> {
    let Some(catalog) = load_catalog(connection)? else {
        return Ok(());
    };
    catalog.annotate_index(index);
    Ok(())
}

/// Validate a v5 index against the sidecar. Missing aliases are never repaired here.
pub(crate) fn validate_sidecar_index(connection: &Connection, index: &Value) -> Result<(), String> {
    let Some(sidecar) = load_catalog(connection)? else {
        return Err("Identity sidecar is incomplete. Restore the matched backup.".into());
    };
    let index_catalog = Catalog::from_index(index)?
        .ok_or("Identity sidecar exists but the index has no identity catalog.")?;
    if sidecar.workspace_id != index_catalog.workspace_id
        || sidecar.owner_id != index_catalog.owner_id
        || sidecar.controller_id != index_catalog.controller_id
    {
        return Err("Identity sidecar and index identity do not match.".into());
    }
    for (kind, ids) in index_catalog.maps() {
        let (_, sidecar_ids) = sidecar
            .maps()
            .into_iter()
            .find(|(sidecar_kind, _)| *sidecar_kind == kind)
            .expect("known identity kind");
        for (legacy, logical) in ids {
            if sidecar_ids.get(legacy) != Some(logical) {
                return Err(format!(
                    "Identity alias {kind}:{legacy} is missing from the sidecar."
                ));
            }
        }
    }
    Ok(())
}

pub(crate) fn sync_index(connection: &Connection, index: &Value) -> Result<(), String> {
    let Some(catalog) = Catalog::from_index(index)? else {
        return Ok(());
    };
    write_catalog(connection, &catalog, None, None, None)
}

pub(crate) fn sync_audit_subject(connection: &Connection, subject: &str) -> Result<(), String> {
    if !subject.starts_with("mcp/") {
        return Ok(());
    }
    let Some(mut catalog) = load_catalog(connection)? else {
        return Ok(());
    };
    catalog.audit_subjects_from([subject.to_owned()])?;
    write_catalog(connection, &catalog, None, None, None)
}

fn receipt_subjects(connection: &Connection) -> Result<Vec<String>, String> {
    let mut statement = connection
        .prepare(
            "SELECT device FROM command_receipts WHERE device LIKE 'mcp/%'
             UNION SELECT device FROM effect_outbox WHERE device LIKE 'mcp/%'",
        )
        .map_err(|e| e.to_string())?;
    statement
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

/// Create and validate the SQLite sidecar after schema 8 is present and the
/// legacy documents have been imported. The transaction publishes the sidecar
/// and the version 5 index together.
pub(crate) fn ensure_database(directory: &Path, connection: &mut Connection) -> Result<(), String> {
    if load_catalog(connection)?.is_some() {
        if let Some((index, _)) = read_index(connection)? {
            if index["version"] == INDEX_VERSION || index["identity"].is_object() {
                validate_sidecar_index(connection, &index)?;
            }
        }
        let subjects = receipt_subjects(connection)?;
        if !subjects.is_empty() {
            let mut catalog = load_catalog(connection)?.expect("checked identity catalog");
            catalog.audit_subjects_from(subjects)?;
            write_catalog(connection, &catalog, None, None, None)?;
        }
        return Ok(());
    }
    let existing = read_index(connection)?;
    if existing.as_ref().is_some_and(|(index, _)| {
        index["version"] != 3 && index["version"] != 4 && index["version"] != INDEX_VERSION
    }) {
        return Err("Unsupported index version during identity migration.".into());
    }
    let routines = read_routines(connection)?;
    let local_identity = Catalog::local(directory);
    let controller =
        controller_from_disk(directory).unwrap_or(local_identity.controller_id.clone());
    let backup = matching_schema_seven_backup(directory)?;
    let backup_database = backup.join("state.sqlite");
    let backup_sha = sha256_file(&backup_database)?;
    let backup_path = backup
        .strip_prefix(directory)
        .unwrap_or(&backup)
        .to_string_lossy()
        .into_owned();
    let source = existing
        .as_ref()
        .map(|(_, text)| text.as_str())
        .unwrap_or("{}");
    let source_sha = format!("{:x}", Sha256::digest(source.as_bytes()));
    let mut index = existing.map(|(value, _)| value).unwrap_or_else(empty_index);
    let migration = Migration {
        backup_path: backup_path.clone(),
        source_index_sha256: source_sha.clone(),
        backup_sha256: backup_sha.clone(),
        migrated_at: crate::lifecycle::now(),
    };
    let mut catalog = migrate_index_with_workspace(
        &mut index,
        &controller,
        &local_identity.workspace_id,
        routines.as_ref(),
        Some(migration.clone()),
    )?;
    catalog.audit_subjects_from(receipt_subjects(connection)?)?;
    let tx = connection.transaction().map_err(|e| e.to_string())?;
    write_catalog(
        &tx,
        &catalog,
        Some(&migration.backup_path),
        Some(&migration.source_index_sha256),
        Some(&migration.backup_sha256),
    )?;
    crate::projection_store::write(&tx, "index.json", &index)?;
    tx.commit().map_err(|e| e.to_string())?;
    validate_sidecar_index(connection, &index)
}

fn empty_index() -> Value {
    json!({
        "version":4,
        "captains":[],
        "projects":[],
        "threads":[],
        "tasks":[],
        "captainScopes":{},
        "captainRemoteScopes":{},
        "captainHomes":{},
        "captainBooks":{},
        "captainLifecycles":{},
        "captainReportPolicies":{},
        "captainReports":[],
        "captainQuestionAnswers":[]
    })
}

/// File-only profiles have no SQLite sidecar. They receive the same catalog in
/// the index, guarded by a verified copy of the pre-migration profile.
pub fn migrate_file_index(directory: &Path, index: &Value) -> Result<Value, String> {
    if directory.join("state.sqlite").exists() {
        return Err("A durable profile must migrate identities through SQLite.".into());
    }
    let source_path = directory.join("index.json");
    let source_bytes = std::fs::read(&source_path).map_err(|e| e.to_string())?;
    let source_sha = format!("{:x}", Sha256::digest(&source_bytes));
    let backup = crate::maintenance::backup_state(directory)?;
    let saved_index = backup.join("index.json");
    if std::fs::read(&saved_index).map_err(|e| e.to_string())? != source_bytes {
        return Err("The identity migration backup does not match index.json.".into());
    }
    for entry in std::fs::read_dir(&backup)
        .map_err(|e| e.to_string())?
        .flatten()
    {
        let saved = entry.path();
        if !saved.is_file() || saved.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let source = directory.join(entry.file_name());
        if source.is_file()
            && std::fs::read(&source).map_err(|e| e.to_string())?
                != std::fs::read(&saved).map_err(|e| e.to_string())?
        {
            return Err(format!(
                "The identity migration backup does not match {}.",
                source.display()
            ));
        }
    }
    let backup_sha = format!("{:x}", Sha256::digest(&source_bytes));
    let backup_path = backup
        .strip_prefix(directory)
        .unwrap_or(&backup)
        .to_string_lossy()
        .into_owned();
    let routines = crate::store::load(&directory.join("routines.json"))?;
    let local_identity = Catalog::local(directory);
    let controller =
        controller_from_disk(directory).unwrap_or(local_identity.controller_id.clone());
    let migration = Migration {
        backup_path,
        source_index_sha256: source_sha,
        backup_sha256: backup_sha,
        migrated_at: crate::lifecycle::now(),
    };
    let mut migrated = index.clone();
    let mut catalog = migrate_index_with_workspace(
        &mut migrated,
        &controller,
        &local_identity.workspace_id,
        routines.as_ref(),
        Some(migration),
    )?;
    catalog.audit_subjects_from(index_receipt_subjects(index))?;
    catalog.annotate_index(&mut migrated);
    crate::store::write_atomic(&source_path, migrated.to_string().as_bytes())
        .map_err(|e| e.to_string())?;
    Ok(migrated)
}

/// Read-only snapshots keep legacy loading available without creating a backup
/// or rewriting the profile.
pub fn migrate_index_in_memory(directory: &Path, index: &Value) -> Result<Value, String> {
    let routines = crate::store::load(&directory.join("routines.json"))?;
    let local_identity = Catalog::local(directory);
    let controller = local_identity.controller_id;
    let mut migrated = index.clone();
    let mut catalog = migrate_index_with_workspace(
        &mut migrated,
        &controller,
        &local_identity.workspace_id,
        routines.as_ref(),
        None,
    )?;
    catalog.audit_subjects_from(index_receipt_subjects(index))?;
    catalog.annotate_index(&mut migrated);
    Ok(migrated)
}

fn index_receipt_subjects(index: &Value) -> Vec<String> {
    index["commandReceipts"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|receipt| [&receipt["deviceId"], &receipt["device"]])
        .filter_map(Value::as_str)
        .filter(|device| device.starts_with("mcp/"))
        .map(str::to_owned)
        .collect()
}

fn matching_schema_seven_backup(directory: &Path) -> Result<PathBuf, String> {
    let mut candidates = Vec::new();
    if let Ok(entries) = std::fs::read_dir(directory.join("backups")) {
        for entry in entries.flatten() {
            let path = entry.path();
            let database = path.join("state.sqlite");
            if !database.is_file() {
                continue;
            }
            let connection =
                Connection::open_with_flags(&database, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                    .map_err(|e| e.to_string())?;
            let version: u32 = connection
                .query_row("PRAGMA user_version", [], |r| r.get(0))
                .map_err(|e| e.to_string())?;
            let integrity: String = connection
                .query_row("PRAGMA integrity_check", [], |r| r.get(0))
                .map_err(|e| e.to_string())?;
            if version == 7 && integrity == "ok" {
                candidates.push(path);
            }
        }
    }
    candidates
        .into_iter()
        .max_by_key(|path| {
            path.metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
        })
        .ok_or_else(|| {
            "No integrity-checked schema 7 backup exists for the identity migration.".into()
        })
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    Ok(format!("{:x}", Sha256::digest(&bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn captain(name: &str) -> Value {
        json!({"name":name,"role":"Role","instructions":"Rules","portrait":0})
    }

    fn fixture() -> Value {
        json!({
            "version":4,
            "captains":[captain("Active"),captain("Archived"),captain("Retired"),captain("Deleted Captain 3")],
            "captainScopes":{"0":[1],"1":[2],"2":[1],"3":[]},
            "captainRemoteScopes":{"0":[{"server":"11111111-1111-4111-8111-111111111111","projectId":9}]},
            "captainHomes":{"0":10,"1":11,"2":12,"3":13},
            "captainBooks":{"0":{"DOC.md":{"text":"v3","revision":3}}},
            "captainLifecycles":{
                "0":{"version":1,"phase":"active"},
                "1":{"version":1,"phase":"archived","archivedAt":1},
                "2":{"version":1,"phase":"retired","retiredAt":2,"formerHome":12},
                "3":{"version":1,"phase":"deleted"}
            },
            "captainReportPolicies":{"0":"react"},
            "captainReports":[{
                "id":"crew-report-1","captain":0,"threadId":11,"runId":"run-1",
                "title":"Done","outcome":"completed","summary":"ok","error":"",
                "createdAt":3,"delivered":false
            }],
            "captainQuestionAnswers":[{
                "id":"crew-answer-1","reportId":"crew-report-1","captain":0,"threadId":11,
                "requestId":"crew-question-1","answer":"yes","actor":"user",
                "createdAt":4,"delivered":false
            }],
            "projects":[
                {"id":1,"name":"Repo","path":"/tmp/shina-identity-repo","machine":"local","hidden":false,"updated":1,"captain":0,"captainThread":10},
                {"id":2,"name":"Repo duplicate","path":"/tmp/shina-identity-repo","machine":"local","hidden":true,"updated":2,"captain":null,"captainThread":null},
                {"id":3,"name":"Remote","path":"/srv/repo","machine":"machine-b","hidden":false,"updated":3,"captain":1,"captainThread":11}
            ],
            "threads":[
                {"id":10,"project":1,"captain":0,"captainHome":true,"crewCaptain":null,"parent":null,
                 "session":"opaque-pi-session","handoffSessions":{"codex":"opaque-acp-session"},"peerOrigin":null},
                {"id":11,"project":1,"captain":0,"captainHome":false,"crewCaptain":0,"parent":10,
                 "session":"opaque-child-session","handoffSessions":{},"peerOrigin":null},
                {"id":12,"project":2,"captain":1,"captainHome":true,"crewCaptain":null,"parent":null,
                 "session":null,"handoffSessions":{},"peerOrigin":{"serverId":"peer","taskId":"peer-task","deviceId":"peer-device"}},
                {"id":13,"project":3,"captain":2,"captainHome":true,"crewCaptain":null,"parent":null,
                 "session":"former-home-session","handoffSessions":{},"peerOrigin":null}
            ],
            "threadViews":[{"id":10,"messages":[]},{"id":11,"messages":[]}],
            "tasks":[{
                "id":"task-1","parent_thread_id":10,"child_thread_id":11,"title":"Work",
                "brief":"Do it","project_id":1,"coordinator_captain":0,"machine":"local",
                "model_target":null,"state":"completed","delivered":true,
                "delivery_id":"delivery-1","result_excerpt":"ok","result":"ok",
                "error_excerpt":"","memory_suggestions":[],"created_at":1,
                "started_at":2,"finished_at":3,"attempt_id":"attempt-1",
                "attempt_message":"marker","prior_deliveries":[]
            }],
            "commandReceipts":[{
                "command_id":"22222222-2222-4222-8222-222222222222",
                "device_id":"mcp/delegate/thread/10","hash":"hash-unchanged",
                "status":"accepted","result":{"thread_id":11},"seq":7
            }]
        })
    }

    fn routines() -> Value {
        json!({"version":1,"next":8,"routines":[{
            "id":7,"captain":0,"home":10,"name":"Routine","prompt":"Run",
            "schedule":{"kind":"every","minutes":15},"missed_policy":"skip",
            "paused":false,"next_at":100,"last_slot":50,
            "last_status":"Scheduled","active_delivery":"routine-7-50"
        }]})
    }

    fn profile_dir(name: &str) -> PathBuf {
        let directory = crate::store::directory().unwrap().join(format!(
            "{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    fn schema_seven_profile(name: &str, index: &Value, routines: &Value) -> PathBuf {
        let directory = profile_dir(name);
        std::fs::write(
            directory.join("server-id"),
            "33333333-3333-4333-8333-333333333333",
        )
        .unwrap();
        let connection = Connection::open(directory.join("state.sqlite")).unwrap();
        for migration in [
            include_str!("migrations/001.sql"),
            include_str!("migrations/002.sql"),
            include_str!("migrations/003.sql"),
            include_str!("migrations/004.sql"),
            include_str!("migrations/005.sql"),
            include_str!("migrations/006.sql"),
            include_str!("migrations/007.sql"),
        ] {
            connection.execute_batch(migration).unwrap();
        }
        connection
            .execute(
                "INSERT INTO metadata(key,value) VALUES('legacy-import-v1',?1)",
                [json!({"fixture":true}).to_string()],
            )
            .unwrap();
        crate::projection_store::write(&connection, "index.json", index).unwrap();
        for (name, value) in [
            ("routines.json", routines.clone()),
            (
                "memory.json",
                json!({"version":1,"next_id":2,"profile":{"name":"","time_zone":"","preferences":""},"captains":{"0":[{"id":"memory-1","text":"Keep this memory","essential":true,"user_controlled":true}]}}),
            ),
            (
                "honcho-outbox.json",
                json!({"version":1,"destination":"http://127.0.0.1","instance":"fixture","next":2,"captains":{"0":"honcho-captain-0"},"pending":[]}),
            ),
        ] {
            connection
                .execute(
                    "INSERT INTO documents(name,payload) VALUES(?1,?2)",
                    params![name, value.to_string()],
                )
                .unwrap();
        }
        connection
            .execute(
                "INSERT INTO command_receipts(device,command_id,hash,payload)
                 VALUES('mcp/captain/0','55555555-5555-4555-8555-555555555555','old-hash',?1)",
                [json!({"command_id":"55555555-5555-4555-8555-555555555555","device_id":"mcp/captain/0","hash":"old-hash","status":"accepted","result":{"frozen":true},"seq":11}).to_string()],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO command_receipts(device,command_id,hash,payload)
                 VALUES('mcp/delegate/thread/99','66666666-6666-4666-8666-666666666666','deleted-child-hash',?1)",
                [json!({"command_id":"66666666-6666-4666-8666-666666666666","device_id":"mcp/delegate/thread/99","hash":"deleted-child-hash","status":"accepted","result":{"frozen":true},"seq":12}).to_string()],
            )
            .unwrap();
        drop(connection);
        directory
    }

    #[test]
    fn migration_maps_every_legacy_entity_and_duplicate_checkout_without_rewrites() {
        let original = fixture();
        let mut migrated = original.clone();
        let routines = routines();
        let catalog = migrate_index(
            &mut migrated,
            "33333333-3333-4333-8333-333333333333",
            Some(&routines),
            None,
        )
        .unwrap();
        assert_eq!(migrated["version"], INDEX_VERSION);
        assert_eq!(catalog.captain_ids.len(), 4);
        assert_eq!(catalog.thread_ids.len(), 4);
        assert_eq!(catalog.project_checkout_ids.len(), 3);
        assert_eq!(catalog.task_ids.len(), 1);
        assert_eq!(catalog.routine_ids.len(), 1);
        assert_eq!(
            catalog.project_checkout_ids["1"], catalog.project_checkout_ids["2"],
            "duplicate local paths share one checkout identity"
        );
        assert_ne!(
            catalog.project_checkout_ids["1"],
            catalog.project_checkout_ids["3"]
        );
        assert_eq!(
            migrated["captains"][3]["captainUid"],
            catalog.captain_ids["3"]
        );
        assert_eq!(
            migrated["projects"][1]["checkoutUid"],
            catalog.project_checkout_ids["1"]
        );
        assert_eq!(
            migrated["threads"][1]["threadUid"],
            catalog.thread_ids["11"]
        );
        assert_eq!(migrated["tasks"][0]["taskUid"], catalog.task_ids["task-1"]);
        assert_eq!(
            migrated["captainReports"][0]["captainUid"],
            catalog.captain_ids["0"]
        );
        assert_eq!(
            migrated["captainQuestionAnswers"][0]["threadUid"],
            catalog.thread_ids["11"]
        );
        assert_eq!(migrated["captainBooks"]["0"]["DOC.md"]["revision"], 3);
        assert_eq!(migrated["threads"][0]["session"], "opaque-pi-session");
        assert_eq!(
            migrated["threads"][0]["handoffSessions"]["codex"],
            "opaque-acp-session"
        );
        assert_eq!(
            migrated["threads"][2]["peerOrigin"],
            original["threads"][2]["peerOrigin"]
        );
        assert_eq!(migrated["tasks"][0]["attempt_id"], "attempt-1");
        assert_eq!(migrated["tasks"][0]["delivery_id"], "delivery-1");
        assert_eq!(migrated["captainReports"][0]["id"], "crew-report-1");
        assert_eq!(migrated["captainQuestionAnswers"][0]["id"], "crew-answer-1");
        assert_eq!(migrated["captainBooks"]["0"]["DOC.md"]["revision"], 3);
        assert_eq!(migrated["commandReceipts"], original["commandReceipts"]);

        let mut replayed = original;
        let again = migrate_index(
            &mut replayed,
            "33333333-3333-4333-8333-333333333333",
            Some(&routines),
            None,
        )
        .unwrap();
        assert_eq!(catalog.captain_ids, again.captain_ids);
        assert_eq!(catalog.project_checkout_ids, again.project_checkout_ids);
        assert_eq!(catalog.thread_ids, again.thread_ids);
        assert_eq!(catalog.task_ids, again.task_ids);
        assert_eq!(catalog.routine_ids, again.routine_ids);
    }

    #[test]
    fn migration_fails_closed_for_missing_references_and_collisions() {
        let mut missing = fixture();
        missing["threads"][1]["parent"] = json!(999);
        assert!(
            migrate_index(
                &mut missing,
                "33333333-3333-4333-8333-333333333333",
                Some(&routines()),
                None,
            )
            .unwrap_err()
            .contains("Missing thread 999")
        );

        let mut collision = fixture();
        collision["threads"][1]["threadUid"] = json!(uuid::Uuid::nil().to_string());
        assert!(
            migrate_index(
                &mut collision,
                "33333333-3333-4333-8333-333333333333",
                Some(&routines()),
                None,
            )
            .unwrap_err()
            .contains("conflicting canonical identity")
        );
    }

    #[test]
    fn equal_numeric_keys_on_two_controllers_never_collide() {
        let mut first = fixture();
        let mut second = fixture();
        let a = migrate_index(
            &mut first,
            "33333333-3333-4333-8333-333333333333",
            Some(&routines()),
            None,
        )
        .unwrap();
        let b = migrate_index(
            &mut second,
            "44444444-4444-4444-8444-444444444444",
            Some(&routines()),
            None,
        )
        .unwrap();
        for kind in ["0", "1", "2", "3"] {
            assert_ne!(a.captain_ids[kind], b.captain_ids[kind]);
        }
        for kind in ["10", "11", "12", "13"] {
            assert_ne!(a.thread_ids[kind], b.thread_ids[kind]);
        }
        for kind in ["1", "2", "3"] {
            assert_ne!(a.project_checkout_ids[kind], b.project_checkout_ids[kind]);
        }
        assert_ne!(a.workspace_id, b.workspace_id);
        assert_ne!(a.owner_id, b.owner_id);
    }

    #[test]
    fn workspace_and_owner_do_not_follow_the_controller_credential_identity() {
        let first = profile_dir("identity-workspace-a");
        let second = profile_dir("identity-workspace-b");
        let controller = "33333333-3333-4333-8333-333333333333";
        std::fs::write(first.join("server-id"), controller).unwrap();
        std::fs::write(second.join("server-id"), controller).unwrap();
        let a = Catalog::local(&first);
        let b = Catalog::local(&second);
        assert_eq!(a.controller_id, b.controller_id);
        assert_ne!(a.workspace_id, b.workspace_id);
        assert_ne!(a.owner_id, b.owner_id);
        assert_ne!(a.owner_id, a.controller_id);
    }

    #[test]
    fn deleted_captain_identity_is_retained_and_a_new_slot_gets_a_different_uid() {
        let mut index = fixture();
        let catalog = migrate_index(
            &mut index,
            "33333333-3333-4333-8333-333333333333",
            Some(&routines()),
            None,
        )
        .unwrap();
        let deleted = catalog.captain_ids["3"].clone();
        let mut with_append = Catalog::from_index(&index).unwrap().unwrap();
        with_append.add_alias(CAPTAIN, "4", "4");
        assert_eq!(with_append.captain_ids["3"], deleted);
        assert_ne!(with_append.captain_ids["4"], deleted);
        assert_eq!(index["captainLifecycles"]["3"]["phase"], "deleted");
    }

    #[test]
    fn audit_subjects_resolve_to_domain_ids_and_remain_outside_devices() {
        let mut migrated = fixture();
        let mut catalog = migrate_index(
            &mut migrated,
            "33333333-3333-4333-8333-333333333333",
            Some(&routines()),
            None,
        )
        .unwrap();
        catalog
            .audit_subjects_from([
                "mcp/captain/0".into(),
                "mcp/delegate/captain/0".into(),
                "mcp/delegate/thread/10".into(),
                "mcp/delegate/thread/99".into(),
            ])
            .unwrap();
        assert_eq!(
            catalog.audit_subjects["mcp/captain/0"].logical_id,
            catalog.captain_ids["0"]
        );
        assert_eq!(
            catalog.audit_subjects["mcp/delegate/thread/10"].logical_id,
            catalog.thread_ids["10"]
        );
        assert_eq!(
            catalog.audit_subjects["mcp/delegate/thread/99"].logical_id, catalog.thread_ids["99"],
            "a deleted child identity remains available to its receipt"
        );
    }

    #[test]
    fn schema_seven_profile_migrates_transactionally_with_a_verified_matching_backup() {
        let original = fixture();
        let routines = routines();
        let directory = schema_seven_profile("identity-complete", &original, &routines);
        let database = Database::open(&directory).unwrap();
        let migrated = database.read("index.json").unwrap().unwrap();
        assert_eq!(migrated["version"], INDEX_VERSION);
        assert_eq!(
            migrated["captains"][3]["captainUid"],
            migrated["identity"]["captainIds"]["3"]
        );
        assert_eq!(
            migrated["identity"]["projectCheckoutIds"]["1"],
            migrated["identity"]["projectCheckoutIds"]["2"]
        );
        assert_eq!(
            migrated["threads"][0]["session"],
            original["threads"][0]["session"]
        );
        assert_eq!(
            migrated["threads"][0]["handoffSessions"],
            original["threads"][0]["handoffSessions"]
        );
        assert_eq!(migrated["tasks"][0]["attempt_id"], "attempt-1");
        assert_eq!(migrated["tasks"][0]["delivery_id"], "delivery-1");
        assert_eq!(migrated["captainReports"][0]["id"], "crew-report-1");
        assert_eq!(migrated["captainQuestionAnswers"][0]["id"], "crew-answer-1");
        assert_eq!(migrated["commandReceipts"], original["commandReceipts"]);
        assert_eq!(
            database.read("memory.json").unwrap().unwrap()["captains"]["0"][0]["text"],
            "Keep this memory"
        );
        assert_eq!(
            database.read("honcho-outbox.json").unwrap().unwrap()["captains"]["0"],
            "honcho-captain-0"
        );
        assert_eq!(
            database.read("routines.json").unwrap().unwrap()["routines"][0]["active_delivery"],
            "routine-7-50"
        );

        let identity = database.identity().unwrap();
        assert_eq!(
            identity["controllerId"],
            "33333333-3333-4333-8333-333333333333"
        );
        assert_eq!(identity["executorId"], identity["controllerId"]);
        assert_ne!(identity["ownerId"], identity["controllerId"]);
        assert_eq!(
            identity["auditSubjects"]["mcp/captain/0"]["logicalId"],
            identity["captainIds"]["0"]
        );
        assert_eq!(
            identity["auditSubjects"]["mcp/delegate/thread/99"]["logicalId"],
            identity["threadIds"]["99"]
        );
        database.test_connection(|connection| {
            let devices: i64 = connection
                .query_row(
                    "SELECT count(*) FROM devices WHERE id LIKE 'mcp/%'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(devices, 0);
            let (backup_path, source_hash, backup_hash): (String, String, String) = connection
                .query_row(
                    "SELECT backup_path,source_index_sha256,backup_sha256 FROM identity_workspace",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .unwrap();
            assert_eq!(
                source_hash,
                format!("{:x}", Sha256::digest(original.to_string().as_bytes()))
            );
            let backup = directory.join(backup_path);
            let saved = Connection::open_with_flags(
                backup.join("state.sqlite"),
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .unwrap();
            assert_eq!(
                saved
                    .query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
                    .unwrap(),
                7
            );
            assert_eq!(
                saved
                    .query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
                    .unwrap(),
                "ok"
            );
            let saved_index = crate::projection_store::read(&saved, "index.json")
                .unwrap()
                .unwrap();
            assert_eq!(saved_index["version"], 4);
            assert_eq!(
                backup_hash,
                sha256_file(&backup.join("state.sqlite")).unwrap()
            );
        });
        let first = identity;
        drop(database);
        let reopened = Database::open(&directory).unwrap();
        assert_eq!(reopened.identity().unwrap(), first);
    }

    #[test]
    fn injected_identity_failure_rolls_back_and_retry_reproduces_the_same_ids() {
        let index = fixture();
        let directory = schema_seven_profile("identity-rollback", &index, &routines());
        let connection = Connection::open(directory.join("state.sqlite")).unwrap();
        connection
            .execute_batch(
                "CREATE TRIGGER reject_identity_index
                 BEFORE INSERT ON projection_records
                 WHEN NEW.document='index.json' AND NEW.scope='$'
                   AND NEW.key='version' AND json_extract(NEW.payload,'$')=5
                 BEGIN SELECT RAISE(ABORT,'injected identity failure'); END;",
            )
            .unwrap();
        drop(connection);
        assert!(Database::open(&directory).is_err());
        let connection = Connection::open(directory.join("state.sqlite")).unwrap();
        let version: u32 = connection
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            version, 8,
            "the schema gate is durable even when identity data rolls back"
        );
        let rows: i64 = connection
            .query_row("SELECT count(*) FROM identity_workspace", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 0);
        let saved = crate::projection_store::read(&connection, "index.json")
            .unwrap()
            .unwrap();
        assert_eq!(saved["version"], 4);
        connection
            .execute("DROP TRIGGER reject_identity_index", [])
            .unwrap();
        drop(connection);

        let database = Database::open(&directory).unwrap();
        let recovered = database.identity().unwrap();
        let mut expected = index;
        let workspace_id = Catalog::local(&directory).workspace_id;
        let mut expected_catalog = migrate_index_with_workspace(
            &mut expected,
            "33333333-3333-4333-8333-333333333333",
            &workspace_id,
            Some(&routines()),
            None,
        )
        .unwrap();
        expected_catalog
            .audit_subjects_from(["mcp/captain/0".into(), "mcp/delegate/thread/99".into()])
            .unwrap();
        for (kind, ids) in expected_catalog.maps() {
            let actual = recovered[map_field(kind)].as_object().unwrap();
            for (legacy, logical) in ids {
                assert_eq!(actual[legacy], *logical, "{kind}:{legacy}");
            }
        }
        drop(database);
        let reopened = Database::open(&directory).unwrap();
        assert_eq!(
            reopened.identity().unwrap()["captainIds"],
            recovered["captainIds"]
        );
        assert_eq!(
            reopened.identity().unwrap()["threadIds"],
            recovered["threadIds"]
        );
    }

    fn map_field(kind: &str) -> &'static str {
        match kind {
            CAPTAIN => "captainIds",
            THREAD => "threadIds",
            PROJECT_CHECKOUT => "projectCheckoutIds",
            TASK => "taskIds",
            ROUTINE => "routineIds",
            _ => panic!("unknown kind"),
        }
    }

    #[test]
    fn missing_reference_never_publishes_identity_data() {
        let mut index = fixture();
        index["threads"][1]["parent"] = json!(999);
        let directory = schema_seven_profile("identity-missing", &index, &routines());
        assert!(Database::open(&directory).is_err());
        let connection = Connection::open(directory.join("state.sqlite")).unwrap();
        let rows: i64 = connection
            .query_row("SELECT count(*) FROM identity_workspace", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 0);
        let saved = crate::projection_store::read(&connection, "index.json")
            .unwrap()
            .unwrap();
        assert_eq!(saved["version"], 4);
    }

    #[test]
    fn empty_state_construction_and_read_only_snapshot_loading_never_write() {
        let absent = crate::store::directory()
            .unwrap()
            .join("identity-empty-state");
        crate::store::with_directory(absent.clone(), || {
            let _ = crate::model::State::new(PathBuf::new(), false);
        });
        assert!(
            !absent.exists(),
            "State::new(PathBuf::new()) must not create a profile"
        );

        let directory = profile_dir("identity-readonly");
        let index = json!({
            "version":4,
            "captains":[captain("Only Captain")],
            "captainScopes":{"0":[]},
            "captainRemoteScopes":{},
            "captainHomes":{},
            "captainBooks":{},
            "captainLifecycles":{"0":{"version":1,"phase":"active"}},
            "captainReportPolicies":{},
            "captainReports":[],
            "captainQuestionAnswers":[],
            "projects":[],
            "threads":[],
            "tasks":[]
        });
        let index_bytes = index.to_string().into_bytes();
        std::fs::write(directory.join("index.json"), &index_bytes).unwrap();
        std::fs::write(
            directory.join("memory.json"),
            json!({"version":1,"next_id":1,"profile":{"name":"","time_zone":"","preferences":""},"captains":{}}).to_string(),
        )
        .unwrap();
        let before = std::fs::read_dir(&directory)
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                (
                    entry.file_name().to_string_lossy().into_owned(),
                    std::fs::read(entry.path()).unwrap(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let mut state = crate::model::State::headless(directory.clone());
        state.load_snapshot();
        assert!(state.storage_error.is_none(), "{:?}", state.storage_error);
        assert_eq!(state.identity.captain_ids.len(), 1);
        let after = std::fs::read_dir(&directory)
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                (
                    entry.file_name().to_string_lossy().into_owned(),
                    std::fs::read(entry.path()).unwrap(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(before, after);
        assert_eq!(
            std::fs::read(directory.join("index.json")).unwrap(),
            index_bytes
        );
    }

    #[test]
    fn newer_index_version_is_rejected_without_overwriting_it() {
        let directory = profile_dir("identity-newer-index");
        let future = json!({"version":6,"captains":[],"projects":[],"threads":[],"tasks":[]});
        std::fs::write(directory.join("index.json"), future.to_string()).unwrap();
        let mut state = crate::model::State::headless(directory.clone());
        state.load_snapshot();
        assert!(
            state
                .storage_error
                .as_deref()
                .is_some_and(|error| error.contains("Unsupported index version"))
        );
        assert_eq!(
            std::fs::read(directory.join("index.json")).unwrap(),
            future.to_string().as_bytes()
        );
    }

    #[test]
    fn file_only_index_gate_migrates_once_after_a_verified_backup() {
        let directory = profile_dir("identity-file-only");
        let mut index = fixture();
        index["captains"][3]["name"] = json!("Deleted Captain 3");
        let source = index.to_string().into_bytes();
        std::fs::write(directory.join("index.json"), &source).unwrap();
        std::fs::write(
            directory.join("routines.json"),
            routines().to_string().as_bytes(),
        )
        .unwrap();
        let migrated = crate::store::load_global(&directory, None)
            .unwrap()
            .unwrap();
        assert_eq!(migrated["version"], INDEX_VERSION);
        let migration = &migrated["identity"]["migration"];
        let backup = directory.join(migration["backupPath"].as_str().unwrap());
        assert_eq!(std::fs::read(backup.join("index.json")).unwrap(), source);
        assert_eq!(
            migration["backupSha256"],
            format!("{:x}", Sha256::digest(&source))
        );
        let second = crate::store::load_global(&directory, None)
            .unwrap()
            .unwrap();
        assert_eq!(
            second["identity"]["captainIds"],
            migrated["identity"]["captainIds"]
        );
    }

    #[test]
    fn newer_sqlite_schema_is_rejected_before_tables_are_touched() {
        let directory = profile_dir("identity-newer-schema");
        let connection = Connection::open(directory.join("state.sqlite")).unwrap();
        connection
            .execute_batch("CREATE TABLE untouched(value TEXT); PRAGMA user_version=9;")
            .unwrap();
        drop(connection);
        let error = match Database::open(&directory) {
            Ok(_) => panic!("newer schema opened"),
            Err(error) => error,
        };
        assert!(error.contains("newer Shina binary"), "{error}");
        let connection = Connection::open(directory.join("state.sqlite")).unwrap();
        let count: i64 = connection
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name='untouched'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }
}
