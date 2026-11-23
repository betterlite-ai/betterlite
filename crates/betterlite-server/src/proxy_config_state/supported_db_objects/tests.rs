#![cfg(test)]

use std::collections::BTreeMap;
use std::io;
use std::str;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::sections::settings::{DbRow, JsonValue, Section, SettingsStore};
use betterlite_error::proxy_enums::supported_db_object_type::SupportedDbObjectType;
use rstest::rstest;
use serde_json::json;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::{MakeWriter, fmt};

use super::{SUPPORTED_DB_OBJECTS, json_type_name, should_load_db_object};

#[test]
fn an_absent_key_loads_every_kind_and_logs_nothing() {
    let (logs, guard) = capture_logs();
    let store = SettingsStore::from(Section::GeneralSettings);
    assert!(should_load_db_object(&store, "models"));
    assert!(should_load_db_object(&store, "agents"));
    drop(guard);
    assert!(logs.contents().is_empty());
}

#[test]
fn a_null_value_loads_every_kind_and_logs_nothing() {
    let (logs, guard) = capture_logs();
    let mut store = SettingsStore::from(Section::GeneralSettings);
    store.load_yaml(BTreeMap::from([(
        SUPPORTED_DB_OBJECTS.to_owned(),
        json!(null),
    )]));
    assert!(should_load_db_object(&store, "models"));
    assert!(should_load_db_object(&store, "agents"));
    drop(guard);
    assert!(logs.contents().is_empty());
}

#[test]
fn an_empty_list_loads_no_kind() {
    let mut store = SettingsStore::from(Section::GeneralSettings);
    store.load_yaml(BTreeMap::from([(
        SUPPORTED_DB_OBJECTS.to_owned(),
        json!([]),
    )]));
    assert!(!should_load_db_object(&store, "models"));
    assert!(!should_load_db_object(&store, "model_cost_map"));
}

#[test]
fn a_list_loads_only_the_kinds_it_names() {
    let (logs, guard) = capture_logs();
    let mut store = SettingsStore::from(Section::GeneralSettings);
    store.load_yaml(BTreeMap::from([(
        SUPPORTED_DB_OBJECTS.to_owned(),
        json!(["models", "mcp"]),
    )]));
    assert!(should_load_db_object(&store, "models"));
    assert!(should_load_db_object(&store, "mcp"));
    assert!(!should_load_db_object(&store, "model_cost_map"));
    assert!(!should_load_db_object(&store, "agents"));
    drop(guard);
    assert!(logs.contents().is_empty());
}

#[test]
fn a_list_entry_that_is_not_a_string_matches_no_kind() {
    let mut store = SettingsStore::from(Section::GeneralSettings);
    store.load_yaml(BTreeMap::from([(
        SUPPORTED_DB_OBJECTS.to_owned(),
        json!(["models", 5, null, ["mcp"]]),
    )]));
    assert!(should_load_db_object(&store, "models"));
    assert!(!should_load_db_object(&store, "mcp"));
    assert!(!should_load_db_object(&store, "5"));
}

#[test]
fn kinds_outside_the_enum_match_by_text() {
    let mut store = SettingsStore::from(Section::GeneralSettings);
    store.load_yaml(BTreeMap::from([(
        SUPPORTED_DB_OBJECTS.to_owned(),
        json!(["agents", "search_tools"]),
    )]));
    assert!(should_load_db_object(&store, "agents"));
    assert!(should_load_db_object(&store, "search_tools"));
    assert!(!should_load_db_object(&store, "models"));
}

#[rstest]
#[case::a_string(json!("models"), "a string")]
#[case::an_object(json!({"models": true}), "an object")]
#[case::a_boolean(json!(true), "a boolean")]
#[case::a_number(json!(5), "a number")]
fn a_value_that_is_not_a_list_warns_and_loads_every_kind(
    #[case] value: JsonValue,
    #[case] kind: &str,
) {
    let (logs, guard) = capture_logs();
    let mut store = SettingsStore::from(Section::GeneralSettings);
    store.load_yaml(BTreeMap::from([(SUPPORTED_DB_OBJECTS.to_owned(), value)]));
    assert!(should_load_db_object(&store, "models"));
    assert!(should_load_db_object(&store, "agents"));
    drop(guard);
    assert!(logs.contents().contains(&format!(
        "supported_db_objects is not a list, got {kind}. Loading all objects."
    )));
}

#[test]
fn json_type_names_name_the_json_types() {
    assert_eq!(json_type_name(&json!(null)), "null");
    assert_eq!(json_type_name(&json!(["models"])), "an array");
    assert_eq!(json_type_name(&json!(true)), "a boolean");
    assert_eq!(json_type_name(&json!(5)), "a number");
    assert_eq!(json_type_name(&json!("mcp")), "a string");
    assert_eq!(json_type_name(&json!({"models": true})), "an object");
}

#[rstest]
#[case::models(SupportedDbObjectType::Models, "models")]
#[case::mcp(SupportedDbObjectType::Mcp, "mcp")]
#[case::guardrails(SupportedDbObjectType::Guardrails, "guardrails")]
#[case::policies(SupportedDbObjectType::Policies, "policies")]
#[case::vector_stores(SupportedDbObjectType::VectorStores, "vector_stores")]
#[case::pass_through_endpoints(
    SupportedDbObjectType::PassThroughEndpoints,
    "pass_through_endpoints"
)]
#[case::prompts(SupportedDbObjectType::Prompts, "prompts")]
#[case::model_cost_map(SupportedDbObjectType::ModelCostMap, "model_cost_map")]
#[case::tools(SupportedDbObjectType::Tools, "tools")]
#[case::config_overrides(SupportedDbObjectType::ConfigOverrides, "config_overrides")]
#[case::websearch_interception_settings(
    SupportedDbObjectType::WebsearchInterceptionSettings,
    "websearch_interception_settings"
)]
fn every_enum_value_matches_through_its_display_text(
    #[case] kind: SupportedDbObjectType,
    #[case] text: &str,
) {
    assert_eq!(kind.to_string(), text);
    let mut store = SettingsStore::from(Section::GeneralSettings);
    store.load_yaml(BTreeMap::from([(
        SUPPORTED_DB_OBJECTS.to_owned(),
        json!([text]),
    )]));
    assert!(should_load_db_object(&store, &kind.to_string()));
}

#[test]
fn a_database_row_value_applies_when_the_config_file_omits_the_key() {
    let mut store = SettingsStore::from(Section::GeneralSettings);
    store.apply_db_row(
        DbRow::GeneralSettings,
        BTreeMap::from([(SUPPORTED_DB_OBJECTS.to_owned(), json!(["models"]))]),
    );
    assert!(should_load_db_object(&store, "models"));
    assert!(!should_load_db_object(&store, "mcp"));
}

#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        str::from_utf8(&bytes)
            .expect("the captured log lines are UTF-8")
            .to_owned()
    }
}

impl io::Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for LogBuffer {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

fn capture_logs() -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(Level::WARN)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}
