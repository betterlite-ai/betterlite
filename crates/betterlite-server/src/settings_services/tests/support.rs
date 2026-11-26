use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_config::sections::settings::{JsonValue, Section};
use betterlite_db::postgres_store::errors::DbError;

use crate::audit_events::identity::CallerIdentity;
use crate::config_overlay_port::{ConfigOverlayPort, ConfigOverlayPortError};
use crate::proxy_config_state::database::{
    ConfigStateDatabase, SectionWrite, merge_section_values,
};
use crate::proxy_config_state::error::ConfigStateError;
use crate::proxy_config_state::snapshot::ConfigWithBaseline;
use crate::proxy_config_state::state::ProxyConfigState;
use crate::settings_services::error::SettingsUpdateError;
use crate::settings_services::keys::SettingKey;
use crate::settings_services::ports::{ConfigAudit, ConfigAuditEntry, LitellmSettingPublisher};
use crate::settings_services::projection::settings_with_schema;
use crate::settings_services::records::SettingsRecord;
use crate::settings_services::response::SettingsUpdateResponse;
use crate::settings_services::schema::{RootSchema, root_schema};
use crate::settings_services::service::{
    SettingsUpdate, SettingsUpdatePorts, update_litellm_setting,
};

// JSON for the tests through serde_json's parser, so no banned object path and no inferred literal type reaches the lint table
macro_rules! json {
    ($($part:tt)*) => {
        $crate::settings_services::tests::support::parse_json(stringify!($($part)*))
    };
}
pub(crate) use json;

pub(crate) const CONFIG_PATH: &str = "/etc/betterlite/config.yaml";

pub(crate) const SETTING: SettingKey = SettingKey::DefaultInternalUserParams;

pub(crate) const SUCCESS_MESSAGE: &str = "Internal user settings updated successfully";

pub(crate) type Events = Arc<Mutex<Vec<&'static str>>>;

pub(crate) fn events() -> Events {
    Arc::new(Mutex::new(Vec::new()))
}

pub(crate) fn recording(events: &Events) -> Vec<&'static str> {
    events.lock().expect("events").clone()
}

pub(crate) fn values(pairs: &[(&str, JsonValue)]) -> BTreeMap<String, JsonValue> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

pub(crate) fn admin() -> CallerIdentity {
    CallerIdentity {
        user_id: Some("admin@example.com".to_owned()),
        api_key: None,
        metadata: None,
        team_metadata: None,
    }
}

// A caller record the shared services read: the schema projection reads values only, and this record declares no defaults
pub(crate) struct TestRecord {
    pub(crate) schema: RootSchema,
    pub(crate) values: BTreeMap<String, JsonValue>,
}

impl SettingsRecord for TestRecord {
    fn schema(&self) -> &RootSchema {
        &self.schema
    }

    fn values(&self) -> BTreeMap<String, JsonValue> {
        self.values.clone()
    }

    fn field_default(&self, _field_name: &str) -> Option<JsonValue> {
        None
    }
}

pub(crate) fn record(values: BTreeMap<String, JsonValue>) -> TestRecord {
    TestRecord {
        schema: RootSchema::default(),
        values,
    }
}

pub(crate) fn schema(raw: &str) -> RootSchema {
    root_schema(&serde_json::from_str::<JsonValue>(raw).unwrap()).unwrap()
}

pub(crate) fn as_json<T>(value: &T) -> JsonValue
where
    T: serde::Serialize,
{
    serde_json::to_value(value).unwrap()
}

pub(crate) fn parse_json(raw: &str) -> JsonValue {
    serde_json::from_str::<JsonValue>(raw).unwrap()
}

pub(crate) fn projection(record: &TestRecord) -> JsonValue {
    as_json(&settings_with_schema(record))
}

// B8379's reload as a scripted fake: it returns the given config or the given failure and records the read
pub(crate) struct ScriptedOverlay {
    events: Events,
    reload: Result<ConfigWithBaseline, ConfigOverlayPortError>,
}

impl ScriptedOverlay {
    pub(crate) fn returning(events: &Events, config: BTreeMap<String, JsonValue>) -> Self {
        let baseline = ConfigWithBaseline::with_baseline(config.clone(), config);
        Self {
            events: Arc::clone(events),
            reload: Ok(baseline),
        }
    }

    pub(crate) fn failing(events: &Events, failure: ConfigOverlayPortError) -> Self {
        Self {
            events: Arc::clone(events),
            reload: Err(failure),
        }
    }
}

#[async_trait]
impl ConfigOverlayPort for ScriptedOverlay {
    async fn reload_config(&self) -> Result<ConfigWithBaseline, ConfigOverlayPortError> {
        self.events.lock().expect("events").push("reload");
        self.reload.clone()
    }

    async fn serve_pass_through_endpoints(
        &self,
        _db_endpoints: &[JsonValue],
    ) -> Result<(), ConfigOverlayPortError> {
        Ok(())
    }
}

// The runtime setting and B065's audit service as one recorder, so both land in the shared event order
pub(crate) struct Recorder {
    events: Events,
    published: Mutex<Vec<(SettingKey, JsonValue)>>,
    audited: Mutex<Vec<ConfigAuditEntry>>,
}

impl Recorder {
    pub(crate) fn new(events: &Events) -> Self {
        Self {
            events: Arc::clone(events),
            published: Mutex::new(Vec::new()),
            audited: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn published(&self) -> Vec<(SettingKey, JsonValue)> {
        self.published.lock().expect("published").clone()
    }

    pub(crate) fn audited(&self) -> Vec<ConfigAuditEntry> {
        self.audited.lock().expect("audited").clone()
    }
}

impl LitellmSettingPublisher for Recorder {
    fn publish(&self, setting: SettingKey, value: &JsonValue) {
        self.events.lock().expect("events").push("publish");
        self.published
            .lock()
            .expect("published")
            .push((setting, value.clone()));
    }
}

#[async_trait]
impl ConfigAudit for Recorder {
    async fn record_config_update(&self, entry: ConfigAuditEntry) {
        self.events.lock().expect("events").push("audit");
        self.audited.lock().expect("audited").push(entry);
    }
}

// B8380's config database in memory: it records the save, keeps the rows and fails on demand
pub(crate) struct RecordingDatabase {
    events: Events,
    sections: Mutex<BTreeMap<String, BTreeMap<String, JsonValue>>>,
    failing: bool,
}

impl RecordingDatabase {
    pub(crate) fn new(events: &Events) -> Self {
        Self {
            events: Arc::clone(events),
            sections: Mutex::new(BTreeMap::new()),
            failing: false,
        }
    }

    pub(crate) fn failing(events: &Events) -> Self {
        Self {
            events: Arc::clone(events),
            sections: Mutex::new(BTreeMap::new()),
            failing: true,
        }
    }

    pub(crate) fn stored(&self, section: Section) -> Option<BTreeMap<String, JsonValue>> {
        self.sections
            .lock()
            .expect("sections")
            .get(section.name())
            .cloned()
    }
}

#[async_trait]
impl ConfigStateDatabase for RecordingDatabase {
    fn seal_environment(
        &self,
        values: &BTreeMap<String, JsonValue>,
    ) -> Result<BTreeMap<String, JsonValue>, ConfigStateError> {
        Ok(values.clone())
    }

    async fn merge_section(
        &self,
        write: &SectionWrite,
    ) -> Result<Option<BTreeMap<String, JsonValue>>, ConfigStateError> {
        self.events.lock().expect("events").push("save");
        if self.failing {
            return Err(ConfigStateError::Database {
                source: DbError::NoDbConnected,
            });
        }
        let existing = self
            .sections
            .lock()
            .expect("sections")
            .get(write.section.name())
            .cloned()
            .unwrap_or_default();
        let merged = merge_section_values(&existing, &write.changed, &write.removed);
        if let Some(values) = &merged {
            self.sections
                .lock()
                .expect("sections")
                .insert(write.section.name().to_owned(), values.clone());
        }
        Ok(merged)
    }

    async fn insert_config(&self, _rows: &[(String, JsonValue)]) -> Result<(), ConfigStateError> {
        Ok(())
    }

    async fn merge_environment_variables(
        &self,
        _sealed: &BTreeMap<String, JsonValue>,
        _deleted: &BTreeSet<String>,
    ) -> Result<(), ConfigStateError> {
        Ok(())
    }
}

// One update wired to the fakes, so a test names only the config and the record it cares about
pub(crate) struct Harness {
    events: Events,
    overlay: ScriptedOverlay,
    recorder: Recorder,
    database: RecordingDatabase,
    state: ProxyConfigState,
}

impl Harness {
    fn assemble(
        events: Events,
        overlay: ScriptedOverlay,
        database: RecordingDatabase,
        state: ProxyConfigState,
    ) -> Self {
        let recorder = Recorder::new(&events);
        Self {
            events,
            overlay,
            recorder,
            database,
            state,
        }
    }

    pub(crate) fn new(config: BTreeMap<String, JsonValue>) -> Self {
        let events = events();
        let overlay = ScriptedOverlay::returning(&events, config);
        let database = RecordingDatabase::new(&events);
        Self::assemble(events, overlay, database, enabled_state())
    }

    pub(crate) fn failing_reload(error: ConfigOverlayPortError) -> Self {
        let events = events();
        let overlay = ScriptedOverlay::failing(&events, error);
        let database = RecordingDatabase::new(&events);
        Self::assemble(events, overlay, database, enabled_state())
    }

    pub(crate) fn failing_database(config: BTreeMap<String, JsonValue>) -> Self {
        let events = events();
        let overlay = ScriptedOverlay::returning(&events, config);
        let database = RecordingDatabase::failing(&events);
        Self::assemble(events, overlay, database, enabled_state())
    }

    pub(crate) fn disabled(config: BTreeMap<String, JsonValue>) -> Self {
        let events = events();
        let overlay = ScriptedOverlay::returning(&events, config);
        let database = RecordingDatabase::new(&events);
        let state = ProxyConfigState::new(PathBuf::from(CONFIG_PATH), false);
        Self::assemble(events, overlay, database, state)
    }

    pub(crate) async fn update(
        &mut self,
        settings: &TestRecord,
    ) -> Result<SettingsUpdateResponse, SettingsUpdateError> {
        let identity = admin();
        let update = SettingsUpdate {
            record: settings,
            setting: SETTING,
            success_message: SUCCESS_MESSAGE,
            admin: &identity,
        };
        let ports = SettingsUpdatePorts {
            overlay: &self.overlay,
            database: Some(&self.database),
            runtime: &self.recorder,
            audit: &self.recorder,
        };
        update_litellm_setting(ports, &mut self.state, &update).await
    }

    pub(crate) fn state_mut(&mut self) -> &mut ProxyConfigState {
        &mut self.state
    }

    pub(crate) fn recording(&self) -> Vec<&'static str> {
        recording(&self.events)
    }

    pub(crate) fn published(&self) -> Vec<(SettingKey, JsonValue)> {
        self.recorder.published()
    }

    pub(crate) fn audited(&self) -> Vec<ConfigAuditEntry> {
        self.recorder.audited()
    }

    pub(crate) fn stored(&self, section: Section) -> Option<BTreeMap<String, JsonValue>> {
        self.database.stored(section)
    }
}

fn enabled_state() -> ProxyConfigState {
    ProxyConfigState::new(PathBuf::from(CONFIG_PATH), true)
}
