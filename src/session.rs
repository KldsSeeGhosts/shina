//! Provider metadata is independent of conversation runtimes. No prompts or credentials are cached.
use crate::backend::{Event, Pi};
use rune::Waker;
use serde_json::Value;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

pub const IDLE_TIMEOUT: Duration = Duration::from_secs(30 * 60);
pub const START_TIMEOUT: Duration = Duration::from_secs(20);
const CATALOG_TTL: Duration = Duration::from_secs(5 * 60);

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct AgentModel {
    pub provider: String,
    pub id: String,
    pub name: String,
    pub thinking_levels: Vec<String>,
    /// The model's real context capacity, from Pi's model metadata. `None` when unknown.
    pub context_window: Option<u64>,
    /// Whether the model's `input` metadata accepts images.
    pub supports_images: Option<bool>,
    #[serde(default)]
    pub service_tiers: Vec<Value>,
}

impl AgentModel {
    pub fn from_harness_value(model: &Value, harness: &str) -> Option<Self> {
        if harness == "pi" {
            return Self::from_value(model);
        }
        if model["provider"] != harness {
            return None;
        }
        Some(Self {
            provider: harness.into(),
            id: model["id"].as_str()?.into(),
            name: model["name"]
                .as_str()
                .or_else(|| model["id"].as_str())?
                .into(),
            thinking_levels: model["thinkingLevels"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|level| level.as_str().map(str::to_owned))
                .collect(),
            context_window: model["contextWindow"].as_u64().filter(|window| *window > 0),
            supports_images: model["input"]
                .as_array()
                .map(|input| input.iter().any(|kind| kind == "image")),
            service_tiers: model["serviceTiers"]
                .as_array()
                .cloned()
                .unwrap_or_default(),
        })
    }
    pub fn from_value(model: &Value) -> Option<Self> {
        if model["provider"] != "cpa" {
            return None;
        }
        let id = model["id"].as_str()?;
        // Pi's model metadata has the same rules as get_available_thinking_levels.
        let thinking_levels = if model["reasoning"] == true {
            ["off", "minimal", "low", "medium", "high", "xhigh", "max"]
                .into_iter()
                .filter(|level| {
                    let mapped = &model["thinkingLevelMap"][*level];
                    !mapped.is_null()
                        || (!matches!(*level, "xhigh" | "max")
                            && model["thinkingLevelMap"].get(*level).is_none())
                })
                .map(str::to_owned)
                .collect()
        } else {
            vec!["off".into()]
        };
        Some(Self {
            provider: "cpa".into(),
            id: id.into(),
            name: model["name"].as_str().unwrap_or(id).into(),
            thinking_levels,
            context_window: model["contextWindow"].as_u64().filter(|window| *window > 0),
            supports_images: model["input"]
                .as_array()
                .map(|input| input.iter().any(|kind| kind == "image")),
            service_tiers: vec![],
        })
    }
}

/// KSG's cached provider probe. One model probe per app start/explicit refresh,
/// with a separate working-directory skill inventory. No thread or prompt is created.
#[derive(Default)]
pub struct NativeCatalog {
    pub models: Vec<AgentModel>,
    pub error: Option<String>,
    pub commands: Vec<Value>,
    pub extensions: bool,
    pub cwd: Option<PathBuf>,
    pub revision: u64,
    pi: Option<Pi>,
    attempted: bool,
    complete: bool,
    models_loaded: bool,
    commands_loaded: bool,
    started: Option<Instant>,
}

impl NativeCatalog {
    pub fn snapshot(&self) -> Value {
        serde_json::json!({"models":self.models,"commands":self.commands,"cwd":self.cwd,
            "extensions":self.extensions,"ready":self.complete,"error":self.error,"loading":self.loading()})
    }
    pub fn loading(&self) -> bool {
        self.pi.is_some()
    }

    pub fn refresh(&mut self, harness: &str, workspace: PathBuf, wake: Waker, force: bool) {
        if self.pi.is_some() {
            return;
        }
        let changed = self.cwd.as_ref() != Some(&workspace);
        if !force && self.attempted && (!changed || self.error.is_some()) {
            return;
        }
        let models = force || !self.attempted;
        self.attempted = true;
        self.revision += 1;
        self.cwd = Some(workspace.clone());
        self.commands.clear();
        self.commands_loaded = false;
        self.models_loaded = !models;
        self.complete = false;
        self.error = None;
        self.started = Some(Instant::now());
        let result = match harness {
            "codex" => crate::codex::discover(workspace, wake, models),
            "claude" => crate::claude::discover(workspace, wake),
            _ => unreachable!(),
        };
        match result {
            Ok(pi) => self.pi = Some(pi),
            Err(error) => self.fail(error),
        }
    }

    pub fn poll(&mut self, harness: &str) -> bool {
        let mut changed = false;
        for _ in 0..256 {
            let Some(event) = self.pi.as_ref().and_then(|pi| pi.events.try_recv().ok()) else {
                break;
            };
            changed = true;
            match event {
                Event::Record(record) if record["type"] == "harness_update" => {
                    if record["field"] == "codexNativeExtensions" {
                        self.extensions = record["value"] == true;
                    }
                }
                Event::Record(record) if record["type"] == "response" => {
                    if record["success"] == false {
                        self.fail(
                            record["error"]
                                .as_str()
                                .unwrap_or("Discovery failed.")
                                .into(),
                        );
                        break;
                    }
                    match record["command"].as_str() {
                        Some("get_available_models") => {
                            self.models = record["data"]["models"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(|model| AgentModel::from_harness_value(model, harness))
                                .collect();
                            self.models_loaded = true;
                            self.revision += 1;
                        }
                        Some("get_commands") => {
                            self.commands = record["data"]["commands"]
                                .as_array()
                                .cloned()
                                .unwrap_or_default();
                            self.commands_loaded = true;
                        }
                        _ => {}
                    }
                    if self.models_loaded && self.commands_loaded {
                        self.pi = None;
                        self.started = None;
                        self.complete = true;
                        self.revision += 1;
                        break;
                    }
                }
                Event::Error(error) => {
                    self.fail(error);
                    break;
                }
                Event::Exited => {
                    self.fail(format!("{harness} stopped during catalog discovery."));
                    break;
                }
                _ => {}
            }
        }
        if self.started.is_some_and(|at| at.elapsed() >= START_TIMEOUT) {
            self.fail(format!("{harness} catalog discovery timed out."));
            changed = true;
        }
        changed
    }

    pub fn commands_ready(&self) -> bool {
        self.complete
    }

    fn fail(&mut self, error: String) {
        self.revision += 1;
        self.error = Some(error);
        self.pi = None;
        self.started = None;
    }
}

#[derive(Default)]
pub struct Catalog {
    pub models: Vec<AgentModel>,
    pub default_model: Option<AgentModel>,
    pub default_thinking: String,
    pub error: Option<String>,
    pi: Option<Pi>,
    started: Option<Instant>,
    loaded: Option<Instant>,
    state_loaded: bool,
    models_loaded: bool,
}

impl Catalog {
    pub fn remaining(&self) -> Option<Duration> {
        self.started
            .map(|at| START_TIMEOUT.saturating_sub(at.elapsed()))
    }

    pub fn refresh(&mut self, workspace: PathBuf, wake: Waker) {
        if self.pi.is_some()
            || (self.error.is_none() && self.loaded.is_some_and(|at| at.elapsed() < CATALOG_TTL))
        {
            return;
        }
        self.error = None;
        self.state_loaded = false;
        self.models_loaded = false;
        self.started = Some(Instant::now());
        self.pi = Some(Pi::discover(workspace, wake));
    }

    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        for _ in 0..256 {
            let Some(event) = self.pi.as_ref().and_then(|pi| pi.events.try_recv().ok()) else {
                break;
            };
            changed = true;
            match event {
                Event::Record(record) if record["type"] == "response" => {
                    if record["success"] == false {
                        self.fail(
                            record["error"]
                                .as_str()
                                .unwrap_or("Model discovery failed.")
                                .into(),
                        );
                        break;
                    }
                    match record["command"].as_str() {
                        Some("get_state") => {
                            self.default_model = AgentModel::from_value(&record["data"]["model"]);
                            self.default_thinking = record["data"]["thinkingLevel"]
                                .as_str()
                                .unwrap_or("")
                                .into();
                            self.state_loaded = true;
                        }
                        Some("get_available_models") => {
                            let Some(models) = record["data"]["models"].as_array() else {
                                self.fail(
                                    "Pi returned an invalid model catalog. Retry to read it."
                                        .into(),
                                );
                                break;
                            };
                            self.models =
                                models.iter().filter_map(AgentModel::from_value).collect();
                            self.models_loaded = true;
                        }
                        _ => {}
                    }
                    if self.state_loaded && self.models_loaded {
                        self.pi = None;
                        self.started = None;
                        self.loaded = Some(Instant::now());
                    }
                }
                Event::Error(error) => self.fail(error),
                Event::Exited => self.fail(
                    "Pi stopped during model discovery. Retry to read the CPA catalog.".into(),
                ),
                _ => {}
            }
        }
        if self.started.is_some_and(|at| at.elapsed() >= START_TIMEOUT) {
            self.fail("Model discovery timed out. Retry to read the CPA catalog.".into());
            changed = true;
        }
        changed
    }

    fn fail(&mut self, error: String) {
        self.error = Some(error);
        self.pi = None;
        self.started = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn native_catalog_caches_models_and_workspace_commands_without_a_thread() {
        for harness in ["codex", "claude"] {
            let (pi, commands, events) = Pi::test_connection();
            let mut catalog = NativeCatalog {
                pi: Some(pi),
                ..Default::default()
            };
            events
                .send(Event::Record(
                    json!({"type":"response","command":"get_available_models","success":true,
                "data":{"models":[{"provider":harness,"id":"fixture","thinkingLevels":["high"]}]}}),
                ))
                .unwrap();
            events
                .send(Event::Record(
                    json!({"type":"response","command":"get_commands","success":true,
                "data":{"commands":[{"name":"$fixture","description":"Project skill"}]}}),
                ))
                .unwrap();
            assert!(catalog.poll(harness));
            assert!(catalog.pi.is_none());
            assert!(catalog.commands_ready());
            assert_eq!(catalog.models[0].thinking_levels, ["high"]);
            assert!(commands.try_recv().is_err());
        }
    }

    #[test]
    fn catalog_discovery_is_independent_of_a_thread_and_releases_its_process() {
        let (pi, commands, events) = Pi::test_connection();
        let mut catalog = Catalog {
            pi: Some(pi),
            ..Default::default()
        };
        for record in [
            json!({"type":"response","command":"get_state","success":true,"data":{
                "model":{"provider":"cpa","id":"default","reasoning":true},"thinkingLevel":"high"
            }}),
            json!({"type":"response","command":"get_available_models","success":true,"data":{"models":[
                {"provider":"direct","id":"hidden"},
                {"provider":"cpa","id":"default","reasoning":true,"thinkingLevelMap":{"minimal":null,"xhigh":"xhigh","max":"max"},"contextWindow":200000,"input":["text","image"]},
                {"provider":"cpa","id":"plain","reasoning":false,"contextWindow":32000,"input":["text"]}
            ]}}),
        ] {
            events.send(Event::Record(record)).unwrap();
        }
        assert!(catalog.poll());
        assert!(catalog.pi.is_none());
        assert!(commands.try_recv().is_err(), "discovery sends no prompts");
        assert_eq!(catalog.models.len(), 2);
        assert_eq!(catalog.default_model.as_ref().unwrap().id, "default");
        assert_eq!(catalog.default_thinking, "high");
        assert_eq!(
            catalog.models[0].thinking_levels,
            ["off", "low", "medium", "high", "xhigh", "max"]
        );
        assert_eq!(catalog.models[1].thinking_levels, ["off"]);
        assert_eq!(catalog.models[0].context_window, Some(200000));
        assert_eq!(catalog.models[0].supports_images, Some(true));
        assert_eq!(catalog.models[1].context_window, Some(32000));
        assert_eq!(catalog.models[1].supports_images, Some(false));
        assert!(catalog.loaded.is_some());
    }

    #[test]
    fn model_metadata_does_not_advertise_unmapped_extended_levels() {
        let model =
            AgentModel::from_value(&json!({"provider":"cpa","id":"reasoning","reasoning":true}))
                .unwrap();
        assert_eq!(
            model.thinking_levels,
            ["off", "minimal", "low", "medium", "high"]
        );
        assert!(AgentModel::from_value(&json!({"provider":"direct","id":"hidden"})).is_none());
    }

    #[test]
    fn model_metadata_keeps_known_capacity_and_vision_support() {
        let model = AgentModel::from_value(&json!({
            "provider":"cpa","id":"vision","name":"Vision","contextWindow":1000000,
            "input":["text","image"]
        }))
        .unwrap();
        assert_eq!(model.context_window, Some(1000000));
        assert_eq!(model.supports_images, Some(true));

        let text = AgentModel::from_value(&json!({
            "provider":"cpa","id":"text","contextWindow":128000,"input":["text"]
        }))
        .unwrap();
        assert_eq!(text.context_window, Some(128000));
        assert_eq!(text.supports_images, Some(false));

        for bad in [
            json!({"provider":"cpa","id":"missing"}),
            json!({"provider":"cpa","id":"zero","contextWindow":0,"input":[]}),
            json!({"provider":"cpa","id":"null","contextWindow":null,"input":null}),
        ] {
            let model = AgentModel::from_value(&bad).unwrap();
            assert_eq!(model.context_window, None, "{bad}");
            assert_eq!(
                model.supports_images,
                bad["input"].as_array().map(|_| false),
                "{bad}"
            );
        }
    }

    #[test]
    fn discovery_failure_keeps_the_previous_catalog_available() {
        let (pi, _commands, events) = Pi::test_connection();
        let mut catalog = Catalog {
            pi: Some(pi),
            models: vec![AgentModel::from_value(&json!({"provider":"cpa","id":"cached"})).unwrap()],
            started: Some(Instant::now()),
            ..Default::default()
        };
        events
            .send(Event::Error("Discovery failed".into()))
            .unwrap();
        catalog.poll();
        assert!(catalog.error.is_some());
        assert!(catalog.pi.is_none());
        assert_eq!(catalog.models[0].id, "cached");
        assert!(catalog.remaining().is_none());
    }
}
