#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use axum::body::to_bytes;
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use betterlite_config::sections::audit_events::Settings;
use betterlite_config::sections::settings::{FieldSource, JsonValue};
use betterlite_error::proxy_error_types::common::CommonProxyErrors;
use secrecy::SecretString;
use serde_json::json;

use super::error::ConfigFieldError;
use super::litellm_fields::UiLitellmField;
use super::ports::{
    ConfigFieldCaller, ConfigFieldDatabase, ConfigFieldRowError, ConfigFieldRows,
    ConfigFieldRuntime, ConfigFieldServices,
};
use super::read::{get_config_general_settings, list_config_general_settings};
use super::records::{
    ConfigFieldDelete, ConfigFieldUpdate, ConfigFieldUpdateResponse, ConfigList, ConfigType,
};
use super::write::{delete_config_general_settings, update_config_general_settings};
use crate::audit_events::callbacks::{
    AuditCallbackDispatcher, AuditCallbackError, AuditCallbackFuture, AuditCallbackRegistry,
    AuditLogCallback, AuditTaskSpawner,
};
use crate::audit_events::enums::{AuditAction, AuditTable};
use crate::audit_events::record::AuditLogRecord;
use crate::audit_events::settings::ConfiguredAuditSettings;
use crate::audit_events::sink::{
    AuditEventSink, AuditLogRepository, AuditRepositoryError, SystemAuditClock,
};
use crate::config_overlay_port::{ConfigOverlayPort, ConfigOverlayPortError};
use crate::plugin_routes::config::PluginRegistry;
use crate::proxy_config_state::database::{ConfigStateDatabase, SectionWrite};
use crate::proxy_config_state::error::ConfigStateError;
use crate::proxy_config_state::snapshot::ConfigWithBaseline;
use crate::proxy_config_state::state::ProxyConfigState;

const ADMIN: &str = "proxy_admin";
const VIEWER: &str = "proxy_admin_viewer";
const CALLER_KEY: &str = "sk-caller";
const PASSTHROUGH: &str = "pass_through_endpoints";

macro_rules! services {
    ($fakes:expr, $state:expr) => {
        ConfigFieldServices {
            state: &mut $state,
            database: Some(&$fakes.database),
            runtime: &$fakes.runtime,
            plugins: &$fakes.plugins,
            audit: &$fakes.sink,
            overlay: &$fakes.overlay,
        }
    };
}

fn object(source: &str) -> BTreeMap<String, JsonValue> {
    serde_json::from_str(source).expect("a JSON object")
}

fn caller(role: &str) -> ConfigFieldCaller {
    ConfigFieldCaller {
        user_id: Some("admin-user".to_owned()),
        user_role: Some(role.to_owned()),
        api_key: Some(SecretString::from(CALLER_KEY.to_owned())),
    }
}

fn update(field_name: &str, field_value: JsonValue) -> ConfigFieldUpdate {
    ConfigFieldUpdate {
        field_name: field_name.to_owned(),
        field_value,
        config_type: ConfigType::GeneralSettings,
    }
}

fn delete(field_name: &str) -> ConfigFieldDelete {
    ConfigFieldDelete {
        field_name: field_name.to_owned(),
        config_type: ConfigType::GeneralSettings,
    }
}

fn state_with_general(source: &str) -> ProxyConfigState {
    let mut state = ProxyConfigState::new(PathBuf::from("/nonexistent/config.yaml"), true);
    state.general_settings.load_yaml(object(source));
    state
}

async fn body_text(response: Response) -> String {
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("a response body");
    String::from_utf8(bytes.to_vec()).expect("utf-8 body")
}

fn bad_request(error: ConfigFieldError) -> String {
    match error {
        ConfigFieldError::BadRequest { detail } => detail,
        ConfigFieldError::Unauthenticated
        | ConfigFieldError::Unprocessable { .. }
        | ConfigFieldError::Unavailable
        | ConfigFieldError::Save(_)
        | ConfigFieldError::Internal => panic!("expected a 400, got {error:?}"),
    }
}

fn listed_entry<'list>(listed: &'list [ConfigList], name: &str) -> &'list ConfigList {
    listed
        .iter()
        .find(|entry| entry.field_name == name)
        .expect("a listed field")
}

#[derive(Default)]
struct StoredRows {
    rows: Mutex<BTreeMap<String, JsonValue>>,
    unavailable: bool,
}

impl StoredRows {
    fn unavailable() -> Self {
        Self {
            rows: Mutex::new(BTreeMap::new()),
            unavailable: true,
        }
    }

    fn set(&self, name: &str, value: JsonValue) {
        self.rows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(name.to_owned(), value);
    }
}

#[async_trait]
impl ConfigFieldRows for StoredRows {
    async fn config_param(&self, name: &str) -> Result<Option<JsonValue>, ConfigFieldRowError> {
        if self.unavailable {
            return Err(ConfigFieldRowError::Unavailable);
        }
        Ok(self
            .rows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(name)
            .cloned())
    }
}

#[derive(Default)]
struct Store {
    inserted: Mutex<Vec<(String, JsonValue)>>,
    writes: Mutex<Vec<SectionWrite>>,
}

impl Store {
    fn inserted_rows(&self) -> Vec<(String, JsonValue)> {
        self.inserted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn section_writes(&self) -> Vec<SectionWrite> {
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl ConfigStateDatabase for Store {
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
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(write.clone());
        Ok(None)
    }

    async fn insert_config(&self, rows: &[(String, JsonValue)]) -> Result<(), ConfigStateError> {
        self.inserted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend(rows.iter().cloned());
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

#[derive(Default)]
struct RecordingRuntime {
    ui: Mutex<BTreeMap<&'static str, JsonValue>>,
    ssrf: Mutex<Vec<BTreeMap<String, JsonValue>>>,
}

impl RecordingRuntime {
    fn ui_value(&self, name: &str) -> JsonValue {
        self.ui
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(name)
            .cloned()
            .unwrap_or(JsonValue::Null)
    }

    fn ssrf_calls(&self) -> Vec<BTreeMap<String, JsonValue>> {
        self.ssrf
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ConfigFieldRuntime for RecordingRuntime {
    fn ui_litellm_field(&self, field: UiLitellmField) -> JsonValue {
        self.ui
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(field.name)
            .cloned()
            .unwrap_or(JsonValue::Null)
    }

    fn set_ui_litellm_field(&self, field: UiLitellmField, value: JsonValue) {
        self.ui
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(field.name, value);
    }

    fn apply_ssrf_settings(&self, general_settings: &BTreeMap<String, JsonValue>) {
        self.ssrf
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(general_settings.clone());
    }
}

struct Overlay {
    config: ConfigWithBaseline,
    reload_error: Option<ConfigOverlayPortError>,
    serve_error: Option<ConfigOverlayPortError>,
    served: Mutex<Vec<Vec<JsonValue>>>,
}

impl Overlay {
    fn with_values(values: BTreeMap<String, JsonValue>) -> Self {
        Self {
            config: ConfigWithBaseline::from_values(values),
            reload_error: None,
            serve_error: None,
            served: Mutex::new(Vec::new()),
        }
    }

    fn served_lists(&self) -> Vec<Vec<JsonValue>> {
        self.served
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl ConfigOverlayPort for Overlay {
    async fn reload_config(&self) -> Result<ConfigWithBaseline, ConfigOverlayPortError> {
        if let Some(error) = self.reload_error {
            return Err(error);
        }
        Ok(self.config.clone())
    }

    async fn serve_pass_through_endpoints(
        &self,
        db_endpoints: &[JsonValue],
    ) -> Result<(), ConfigOverlayPortError> {
        self.served
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(db_endpoints.to_vec());
        if let Some(error) = self.serve_error {
            return Err(error);
        }
        Ok(())
    }
}

#[derive(Default)]
struct RecordingRepository {
    inserted: Mutex<Vec<AuditLogRecord>>,
}

#[async_trait]
impl AuditLogRepository for RecordingRepository {
    fn available(&self) -> bool {
        true
    }

    async fn insert_audit_log(&self, record: &AuditLogRecord) -> Result<(), AuditRepositoryError> {
        self.inserted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(record.clone());
        Ok(())
    }
}

struct NoCallbacks;

impl AuditCallbackRegistry for NoCallbacks {
    fn resolve(&self, _name: &str) -> Option<Arc<dyn AuditLogCallback>> {
        None
    }

    fn build_dedicated_s3_logger(
        &self,
        _params: &JsonValue,
    ) -> Result<Arc<dyn AuditLogCallback>, AuditCallbackError> {
        Err(AuditCallbackError::Failed)
    }
}

// audit logging is on and no callback is configured, so only the row write matters here
struct DroppingSpawner;

impl AuditTaskSpawner for DroppingSpawner {
    fn spawn(&self, task: AuditCallbackFuture) {
        drop(task);
    }
}

fn audit_sink(repository: &Arc<RecordingRepository>) -> AuditEventSink {
    let settings = ConfiguredAuditSettings {
        config: Settings {
            store_audit_logs: Some(true),
            store_audit_logs_env: None,
            audit_log_callbacks: Vec::new(),
            s3_audit_callback_params: None,
        },
        callbacks: Vec::new(),
    };
    let dispatcher = AuditCallbackDispatcher::new(Arc::new(NoCallbacks), Arc::new(DroppingSpawner));
    let repository_port: Arc<dyn AuditLogRepository> =
        Arc::<RecordingRepository>::clone(repository);
    AuditEventSink {
        settings: Arc::new(settings),
        callbacks: Arc::new(dispatcher),
        repository: repository_port,
        clock: Arc::new(SystemAuditClock),
        new_id: Arc::new(|| "audit-id".to_owned()),
    }
}

struct Fakes {
    rows: Arc<StoredRows>,
    store: Arc<Store>,
    database: ConfigFieldDatabase,
    runtime: RecordingRuntime,
    plugins: PluginRegistry,
    overlay: Overlay,
    repository: Arc<RecordingRepository>,
    sink: AuditEventSink,
}

fn fixtures() -> Fakes {
    let rows = Arc::new(StoredRows::default());
    let store = Arc::new(Store::default());
    let rows_port: Arc<dyn ConfigFieldRows> = Arc::<StoredRows>::clone(&rows);
    let store_port: Arc<dyn ConfigStateDatabase> = Arc::<Store>::clone(&store);
    let repository = Arc::new(RecordingRepository::default());
    let sink = audit_sink(&repository);
    Fakes {
        rows,
        store,
        database: ConfigFieldDatabase {
            rows: rows_port,
            store: store_port,
        },
        runtime: RecordingRuntime::default(),
        plugins: PluginRegistry::default(),
        overlay: Overlay::with_values(BTreeMap::new()),
        repository,
        sink,
    }
}

impl Fakes {
    fn audit_rows(&self) -> Vec<AuditLogRecord> {
        self.repository
            .inserted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[tokio::test]
async fn info_reports_the_value_source_and_redaction() {
    let fakes = fixtures();
    let mut state = state_with_general(
        r#"{"max_parallel_requests": 5, "database_url": "postgres://user:pass@host/db", "plugins": [{"name": "p", "url": "https://example.test", "plugin_key": "sk-stored"}]}"#,
    );
    let info = get_config_general_settings(
        services!(fakes, state),
        &caller(ADMIN),
        "max_parallel_requests",
    )
    .await
    .expect("the field info");
    assert_eq!(info.field_value, json!(5));
    assert_eq!(info.source, FieldSource::Config);
    assert!(!info.editable);
    let secret =
        get_config_general_settings(services!(fakes, state), &caller(VIEWER), "database_url")
            .await
            .expect("the redacted info");
    assert_eq!(secret.field_value, json!("REDACTED"));
    let plugins = get_config_general_settings(services!(fakes, state), &caller(ADMIN), "plugins")
        .await
        .expect("the plugin info");
    assert_eq!(plugins.field_value[0]["plugin_key"], json!("***"));
}

#[tokio::test]
async fn info_reads_a_resource_list_from_the_writer_row() {
    let fakes = fixtures();
    fakes.rows.set(
        "general_settings",
        json!({"pass_through_endpoints": [{"path": "/bria", "target": "https://example.test"}]}),
    );
    let mut state = state_with_general("{}");
    let info = get_config_general_settings(services!(fakes, state), &caller(ADMIN), PASSTHROUGH)
        .await
        .expect("the field info");
    assert_eq!(info.field_value[0]["path"], json!("/bria"));
}

#[tokio::test]
async fn info_refuses_an_unknown_and_an_unset_field() {
    let fakes = fixtures();
    let mut state = state_with_general("{}");
    let unknown =
        get_config_general_settings(services!(fakes, state), &caller(ADMIN), "not_a_field")
            .await
            .expect_err("an unknown field");
    assert_eq!(bad_request(unknown), "Invalid field=not_a_field passed in.");
    let unset = get_config_general_settings(
        services!(fakes, state),
        &caller(ADMIN),
        "max_parallel_requests",
    )
    .await
    .expect_err("an unset field");
    assert_eq!(
        bad_request(unset),
        "Field name=max_parallel_requests is not set"
    );
}

#[tokio::test]
async fn update_writes_the_field_applies_ssrf_and_audits() {
    let fakes = fixtures();
    let mut state = state_with_general("{}");
    let written = update_config_general_settings(
        services!(fakes, state),
        &caller(ADMIN),
        &update("max_parallel_requests", json!(7)),
    )
    .await
    .expect("the update");
    match written {
        ConfigFieldUpdateResponse::Row(row) => {
            assert_eq!(row.param_name, "general_settings");
            assert_eq!(row.param_value["max_parallel_requests"], json!(7));
        }
        ConfigFieldUpdateResponse::Ui(body) => panic!("a row write, got {body:?}"),
    }
    assert_eq!(fakes.store.inserted_rows().len(), 1);
    assert_eq!(fakes.runtime.ssrf_calls().len(), 1);
    let audit = fakes.audit_rows();
    assert_eq!(audit[0].action, AuditAction::Updated);
    assert_eq!(audit[0].table_name, AuditTable::Config);
    assert_eq!(audit[0].object_id, "general_settings");
}

#[tokio::test]
async fn update_refuses_an_unknown_field_and_a_wrong_type() {
    let fakes = fixtures();
    let mut state = state_with_general("{}");
    let unknown = update_config_general_settings(
        services!(fakes, state),
        &caller(ADMIN),
        &update("not_a_field", json!(1)),
    )
    .await
    .expect_err("an unknown field");
    assert_eq!(bad_request(unknown), "Invalid field=not_a_field passed in.");
    let wrong_type = update_config_general_settings(
        services!(fakes, state),
        &caller(ADMIN),
        &update("max_parallel_requests", json!("seven")),
    )
    .await
    .expect_err("a wrong type");
    assert_eq!(
        bad_request(wrong_type),
        "Invalid type of field value=<class 'str'> passed in."
    );
    assert!(fakes.store.inserted_rows().is_empty());
}

#[tokio::test]
async fn update_preserves_the_stored_plugin_key() {
    let fakes = fixtures();
    fakes.rows.set(
        "general_settings",
        json!({"plugins": [{"name": "p", "url": "https://one.test", "plugin_key": "sk-stored"}]}),
    );
    let mut state = state_with_general("{}");
    update_config_general_settings(
        services!(fakes, state),
        &caller(ADMIN),
        &update(
            "plugins",
            json!([
                {"name": "p", "url": "https://one.test", "plugin_key": "***"},
                {"name": "q", "url": "https://two.test"}
            ]),
        ),
    )
    .await
    .expect("the plugins update");
    let inserted = fakes.store.inserted_rows();
    let written = inserted[0].1.get("plugins").expect("the written plugins");
    assert_eq!(written[0]["plugin_key"], json!("sk-stored"));
    assert_eq!(written[1].get("plugin_key"), None);
    let registry = fakes
        .plugins
        .plugins
        .read()
        .unwrap_or_else(PoisonError::into_inner);
    assert_eq!(registry.len(), 2);
}

#[tokio::test]
async fn update_refuses_a_config_owned_field() {
    let fakes = fixtures();
    let mut state = state_with_general(r#"{"max_parallel_requests": 5}"#);
    let refusal = update_config_general_settings(
        services!(fakes, state),
        &caller(ADMIN),
        &update("max_parallel_requests", json!(7)),
    )
    .await
    .expect_err("a config-owned field");
    assert!(matches!(refusal, ConfigFieldError::Save(_)));
    assert!(fakes.store.inserted_rows().is_empty());
}

#[tokio::test]
async fn alerting_args_reports_every_invalid_argument() {
    let fakes = fixtures();
    let mut state = state_with_general("{}");
    let refusal = update_config_general_settings(
        services!(fakes, state),
        &caller(ADMIN),
        &update(
            "alerting_args",
            json!({"daily_report_frequency": 1.5, "spend_anomaly_multiplier": 0}),
        ),
    )
    .await
    .expect_err("invalid alerting args");
    assert_eq!(
        bad_request(refusal),
        "Invalid alerting_args: daily_report_frequency: Input should be a valid integer, got a number with a fractional part; spend_anomaly_multiplier: Input should be greater than 0"
    );
}

#[tokio::test]
async fn alerting_args_accepts_a_whole_float_for_an_int_field() {
    let fakes = fixtures();
    let mut state = state_with_general("{}");
    update_config_general_settings(
        services!(fakes, state),
        &caller(ADMIN),
        &update(
            "alerting_args",
            json!({"daily_report_frequency": 2.0, "spend_anomaly_multiplier": 3.0}),
        ),
    )
    .await
    .expect("whole floats are ints to pydantic");
}

#[tokio::test]
async fn gates_name_the_role_only_on_delete_and_list() {
    let fakes = fixtures();
    let mut state = state_with_general("{}");
    let update_refusal = update_config_general_settings(
        services!(fakes, state),
        &caller(VIEWER),
        &update("max_parallel_requests", json!(1)),
    )
    .await
    .expect_err("a viewer cannot update");
    assert_eq!(
        bad_request(update_refusal),
        CommonProxyErrors::NotAllowedAccess.as_str()
    );
    let delete_refusal = delete_config_general_settings(
        services!(fakes, state),
        &caller(VIEWER),
        &delete("max_parallel_requests"),
    )
    .await
    .expect_err("a viewer cannot delete");
    assert_eq!(
        bad_request(delete_refusal),
        format!(
            "{}, your role=proxy_admin_viewer",
            CommonProxyErrors::NotAllowedAccess.as_str()
        )
    );
    let list_refusal =
        list_config_general_settings(services!(fakes, state), &caller("internal_user"))
            .await
            .expect_err("an internal user cannot list");
    assert_eq!(
        bad_request(list_refusal),
        format!(
            "{}, your role=internal_user",
            CommonProxyErrors::NotAllowedAccess.as_str()
        )
    );
}

#[tokio::test]
async fn delete_pops_the_field_and_audits_deleted() {
    let fakes = fixtures();
    fakes
        .rows
        .set("general_settings", json!({"max_parallel_requests": 5}));
    let mut state = state_with_general("{}");
    let written = delete_config_general_settings(
        services!(fakes, state),
        &caller(ADMIN),
        &delete("max_parallel_requests"),
    )
    .await
    .expect("the delete");
    match written {
        ConfigFieldUpdateResponse::Row(row) => {
            assert_eq!(row.param_value.get("max_parallel_requests"), None);
        }
        ConfigFieldUpdateResponse::Ui(body) => panic!("a row delete, got {body:?}"),
    }
    let audit = fakes.audit_rows();
    assert_eq!(audit[0].action, AuditAction::Deleted);
    assert_eq!(audit[0].object_id, "general_settings");
}

#[tokio::test]
async fn delete_refuses_a_field_that_is_not_stored() {
    let fakes = fixtures();
    let mut state = state_with_general("{}");
    let refusal = delete_config_general_settings(
        services!(fakes, state),
        &caller(ADMIN),
        &delete("max_parallel_requests"),
    )
    .await
    .expect_err("a missing row");
    assert_eq!(
        bad_request(refusal),
        "Field name=max_parallel_requests not in config"
    );
}

#[tokio::test]
async fn a_missing_database_answers_db_not_connected() {
    let fakes = fixtures();
    let mut state = state_with_general("{}");
    let services = ConfigFieldServices {
        state: &mut state,
        database: None,
        runtime: &fakes.runtime,
        plugins: &fakes.plugins,
        audit: &fakes.sink,
        overlay: &fakes.overlay,
    };
    let refusal = list_config_general_settings(services, &caller(ADMIN))
        .await
        .expect_err("no database");
    assert_eq!(
        bad_request(refusal),
        CommonProxyErrors::DbNotConnectedError.as_str()
    );
}

#[tokio::test]
async fn list_returns_the_catalog_then_the_ui_fields() {
    let fakes = fixtures();
    fakes
        .rows
        .set("general_settings", json!({"store_model_in_db": true}));
    fakes
        .rows
        .set("litellm_settings", json!({"budget_rollover": true}));
    let mut state = state_with_general(r#"{"max_parallel_requests": 5}"#);
    let listed = list_config_general_settings(services!(fakes, state), &caller(ADMIN))
        .await
        .expect("the config list");
    assert_eq!(listed.len(), 40);
    assert_eq!(listed[0].field_name, "max_in_flight_requests_per_worker");
    assert_eq!(listed[34].field_name, "budget_exceeded_throttle_percentage");
    let parallel = listed_entry(&listed, "max_parallel_requests");
    assert_eq!(parallel.field_value, json!(5));
    assert_eq!(parallel.stored_in_db, Some(false));
    let pass_through = listed_entry(&listed, PASSTHROUGH);
    assert_eq!(
        pass_through
            .nested_fields
            .as_ref()
            .expect("nested fields")
            .len(),
        12
    );
    let ttl = listed_entry(&listed, "anthropic_prompt_caching_ttl");
    assert_eq!(ttl.field_options, Some(vec!["5m", "1h"]));
    assert_eq!(ttl.field_tab, Some("prompt_caching"));
    let rollover = listed_entry(&listed, "budget_rollover");
    assert_eq!(rollover.stored_in_db, Some(true));
    assert_eq!(rollover.field_default_value, json!(false));
}

#[tokio::test]
async fn list_carries_the_max_ui_session_budget_description() {
    let fakes = fixtures();
    let mut state = state_with_general("{}");
    let listed = list_config_general_settings(services!(fakes, state), &caller(ADMIN))
        .await
        .expect("the config list");
    let budget = listed_entry(&listed, "max_ui_session_budget");
    assert_eq!(
        budget.field_description,
        "USD spend cap for each dashboard login session; covers LLM calls made from the dashboard such as the playground and auto router Test Connection. Each login starts a fresh session with this budget. Clearing restores the $1 default."
    );
}

#[tokio::test]
async fn pass_through_reserve_update_serves_the_written_list() {
    let fakes = fixtures();
    let mut state = state_with_general("{}");
    update_config_general_settings(
        services!(fakes, state),
        &caller(ADMIN),
        &update(
            PASSTHROUGH,
            json!([{"path": "/bria", "target": "https://example.test"}]),
        ),
    )
    .await
    .expect("the pass-through update");
    let served = fakes.overlay.served_lists();
    assert_eq!(served.len(), 1);
    assert_eq!(served[0][0]["path"], json!("/bria"));
    assert_eq!(fakes.audit_rows()[0].action, AuditAction::Updated);
}

#[tokio::test]
async fn pass_through_reserve_delete_serves_the_remaining_list() {
    let fakes = fixtures();
    fakes.rows.set(
        "general_settings",
        json!({
            "max_parallel_requests": 5,
            "pass_through_endpoints": [{"path": "/bria", "target": "https://example.test"}]
        }),
    );
    let mut state = state_with_general("{}");
    delete_config_general_settings(
        services!(fakes, state),
        &caller(ADMIN),
        &delete(PASSTHROUGH),
    )
    .await
    .expect("the pass-through delete");
    let served = fakes.overlay.served_lists();
    assert_eq!(served.len(), 1);
    assert!(served[0].is_empty());
    let inserted = fakes.store.inserted_rows();
    assert_eq!(inserted[0].1.get("pass_through_endpoints"), None);
    assert_eq!(fakes.audit_rows()[0].action, AuditAction::Deleted);
}

#[tokio::test]
async fn pass_through_reserve_database_unavailable_answers_503_without_an_audit() {
    let mut fakes = fixtures();
    fakes.overlay.serve_error = Some(ConfigOverlayPortError::DatabaseUnavailable);
    let mut state = state_with_general("{}");
    let refusal = update_config_general_settings(
        services!(fakes, state),
        &caller(ADMIN),
        &update(
            PASSTHROUGH,
            json!([{"path": "/bria", "target": "https://example.test"}]),
        ),
    )
    .await
    .expect_err("the re-serve is unavailable");
    let response = refusal.into_response();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(body_text(response).await.contains("no_db_connection"));
    assert_eq!(fakes.store.inserted_rows().len(), 1);
    assert!(fakes.audit_rows().is_empty());
    fakes.overlay.serve_error = Some(ConfigOverlayPortError::PassThrough);
    let serve_failure = update_config_general_settings(
        services!(fakes, state),
        &caller(ADMIN),
        &update(
            PASSTHROUGH,
            json!([{"path": "/bria", "target": "https://example.test"}]),
        ),
    )
    .await
    .expect_err("the re-serve failed");
    let response = serve_failure.into_response();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(fakes.audit_rows().is_empty());
}

#[tokio::test]
async fn pass_through_reserve_row_unavailable_answers_503_without_an_audit() {
    let mut fakes = fixtures();
    fakes.rows = Arc::new(StoredRows::unavailable());
    let rows_port: Arc<dyn ConfigFieldRows> = Arc::<StoredRows>::clone(&fakes.rows);
    fakes.database.rows = rows_port;
    let mut state = state_with_general("{}");
    let refusal = update_config_general_settings(
        services!(fakes, state),
        &caller(ADMIN),
        &update(
            PASSTHROUGH,
            json!([{"path": "/bria", "target": "https://example.test"}]),
        ),
    )
    .await
    .expect_err("the row read is unavailable");
    let response = refusal.into_response();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(body_text(response).await.contains("no_db_connection"));
    assert!(fakes.store.inserted_rows().is_empty());
    assert!(fakes.audit_rows().is_empty());
}

mod corpus;
mod ui_reload;
mod validators;
