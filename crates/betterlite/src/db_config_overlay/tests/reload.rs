use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::sections::settings::JsonValue;
use betterlite_server::bucket_config::error::BucketConfigError;
use rstest::rstest;
use tempfile::TempDir;

use super::support::{
    RecordingRuntime, bucket_source, database, environment, file_source, json, state, write_config,
};
use crate::config_boot::test_support::{capture_logs, secret_lookup};
use crate::db_config_overlay::environ::Resolver;
use crate::db_config_overlay::error::OverlayError;
use crate::db_config_overlay::reload::reload_config;
use crate::db_config_overlay::source::ConfigSource;

const BUCKET_ENV: [(&str, &str); 2] = [
    ("LITELLM_CONFIG_BUCKET_NAME", "configs"),
    ("LITELLM_CONFIG_BUCKET_OBJECT_KEY", "proxy.yaml"),
];

async fn reload_file(
    yaml: &str,
    env: &mut BTreeMap<String, String>,
) -> Result<JsonValue, OverlayError> {
    let directory = TempDir::new().unwrap();
    let path = write_config(directory.path(), yaml);
    let mut overlay = state(false, &Arc::new(RecordingRuntime::default()));
    let reloaded = reload_config(&mut overlay, &file_source(&path), None, env).await?;
    Ok(reloaded.values.into_iter().collect::<JsonValue>())
}

#[tokio::test]
async fn reload_returns_the_file_config_as_its_own_baseline() {
    let directory = TempDir::new().unwrap();
    let path = write_config(
        directory.path(),
        "model_list:\n  - model_name: gpt\ngeneral_settings:\n  master_key: sk-file\n",
    );
    let mut overlay = state(false, &Arc::new(RecordingRuntime::default()));
    let reloaded = reload_config(
        &mut overlay,
        &file_source(&path),
        None,
        &mut BTreeMap::new(),
    )
    .await
    .unwrap();
    assert_eq!(
        reloaded.values.clone().into_iter().collect::<JsonValue>(),
        json!({"model_list": [{"model_name": "gpt"}], "general_settings": {"master_key": "sk-file"}})
    );
    assert_eq!(reloaded.baseline, Some(reloaded.values.clone()));
    assert_eq!(overlay.config.snapshot(), reloaded.values);
    assert_eq!(
        overlay.config.general_settings.get("master_key"),
        Some(&json!("sk-file"))
    );
}

#[tokio::test]
async fn reload_without_a_path_returns_the_empty_sections() {
    let source = ConfigSource {
        config_file_path: None,
        bucket_reader: None,
    };
    let mut overlay = state(false, &Arc::new(RecordingRuntime::default()));
    let reloaded = reload_config(&mut overlay, &source, None, &mut BTreeMap::new())
        .await
        .unwrap();
    assert_eq!(
        reloaded.values.into_iter().collect::<JsonValue>(),
        json!({"model_list": [], "general_settings": {}, "router_settings": {}, "litellm_settings": {}})
    );
}

#[tokio::test]
async fn reload_refuses_a_missing_config_file() {
    let directory = TempDir::new().unwrap();
    let missing = directory.path().join("absent.yaml");
    let mut overlay = state(false, &Arc::new(RecordingRuntime::default()));
    let error = reload_config(
        &mut overlay,
        &file_source(&missing),
        None,
        &mut BTreeMap::new(),
    )
    .await
    .unwrap_err();
    assert!(matches!(&error, OverlayError::ConfigFileMissing { path } if path == &missing));
    assert_eq!(
        error.to_string(),
        format!("Config file not found: {}", missing.display())
    );
}

#[rstest]
#[case::empty("")]
#[case::null("~\n")]
#[tokio::test]
async fn reload_refuses_an_empty_config_file(#[case] yaml: &str) {
    let error = reload_file(yaml, &mut BTreeMap::new()).await.unwrap_err();
    assert!(matches!(error, OverlayError::ConfigEmpty));
}

#[rstest]
#[case::list("- a\n")]
#[case::text("just text\n")]
#[case::number("7\n")]
#[case::flag("true\n")]
#[tokio::test]
async fn reload_refuses_a_config_file_that_is_not_a_mapping(#[case] yaml: &str) {
    let error = reload_file(yaml, &mut BTreeMap::new()).await.unwrap_err();
    assert!(matches!(error, OverlayError::ConfigNotMapping { .. }));
}

#[tokio::test]
async fn reload_refuses_a_config_file_that_is_not_yaml() {
    let error = reload_file("general_settings: [unclosed\n", &mut BTreeMap::new())
        .await
        .unwrap_err();
    assert!(matches!(error, OverlayError::ConfigYaml { .. }));
}

#[tokio::test]
async fn reload_refuses_a_config_path_that_is_a_directory() {
    let directory = TempDir::new().unwrap();
    let mut overlay = state(false, &Arc::new(RecordingRuntime::default()));
    let error = reload_config(
        &mut overlay,
        &file_source(directory.path()),
        None,
        &mut BTreeMap::new(),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, OverlayError::ReadConfigFile { .. }));
}

#[tokio::test]
async fn reload_reads_the_bucket_when_its_name_is_set() {
    let source = bucket_source(&[("proxy.yaml", "general_settings:\n  master_key: sk-bucket\n")]);
    let mut overlay = state(false, &Arc::new(RecordingRuntime::default()));
    let mut env = environment(&BUCKET_ENV);
    let reloaded = reload_config(&mut overlay, &source, None, &mut env)
        .await
        .unwrap();
    assert_eq!(
        reloaded.values.into_iter().collect::<JsonValue>(),
        json!({"general_settings": {"master_key": "sk-bucket"}})
    );
}

#[tokio::test]
async fn reload_from_a_bucket_without_the_object_loads_nothing() {
    let source = bucket_source(&[("other.yaml", "general_settings: {}\n")]);
    let mut overlay = state(false, &Arc::new(RecordingRuntime::default()));
    let error = reload_config(&mut overlay, &source, None, &mut environment(&BUCKET_ENV))
        .await
        .unwrap_err();
    assert!(matches!(error, OverlayError::NothingLoaded));
    assert_eq!(
        error.to_string(),
        "Unable to load config from given source."
    );
}

#[tokio::test]
async fn reload_from_a_bucket_without_an_object_key_fails() {
    let source = bucket_source(&[]);
    let mut overlay = state(false, &Arc::new(RecordingRuntime::default()));
    let mut env = environment(&[("LITELLM_CONFIG_BUCKET_NAME", "configs")]);
    let error = reload_config(&mut overlay, &source, None, &mut env)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        OverlayError::Bucket(BucketConfigError::MissingObjectKey)
    ));
}

#[tokio::test]
async fn reload_from_a_bucket_without_a_reader_fails() {
    let directory = TempDir::new().unwrap();
    let path = write_config(directory.path(), "general_settings: {}\n");
    let mut overlay = state(false, &Arc::new(RecordingRuntime::default()));
    let error = reload_config(
        &mut overlay,
        &file_source(&path),
        None,
        &mut environment(&BUCKET_ENV),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, OverlayError::NoBucketReader));
}

#[tokio::test]
async fn reload_with_an_empty_bucket_name_reads_the_file() {
    let directory = TempDir::new().unwrap();
    let path = write_config(
        directory.path(),
        "general_settings:\n  master_key: sk-file\n",
    );
    let mut overlay = state(false, &Arc::new(RecordingRuntime::default()));
    let mut env = environment(&[("LITELLM_CONFIG_BUCKET_NAME", "")]);
    let reloaded = reload_config(&mut overlay, &file_source(&path), None, &mut env)
        .await
        .unwrap();
    assert_eq!(
        reloaded.values.into_iter().collect::<JsonValue>(),
        json!({"general_settings": {"master_key": "sk-file"}})
    );
}

#[rstest]
#[case::enabled(true, json!({"general_settings": {"master_key": "sk-file", "max_parallel_requests": 4}}))]
#[case::disabled(false, json!({"general_settings": {"master_key": "sk-file"}}))]
#[tokio::test]
async fn reload_overlays_the_rows_only_when_models_live_in_the_database(
    #[case] store_model_in_db: bool,
    #[case] expected: JsonValue,
) {
    let directory = TempDir::new().unwrap();
    let path = write_config(
        directory.path(),
        "general_settings:\n  master_key: sk-file\n",
    );
    let mut overlay = state(store_model_in_db, &Arc::new(RecordingRuntime::default()));
    let rows = database(&[("general_settings", json!({"max_parallel_requests": 4}))]);
    let reloaded = reload_config(
        &mut overlay,
        &file_source(&path),
        Some(&rows),
        &mut BTreeMap::new(),
    )
    .await
    .unwrap();
    assert_eq!(reloaded.values.into_iter().collect::<JsonValue>(), expected);
}

#[tokio::test]
async fn reload_drops_the_rows_of_the_previous_reload() {
    let directory = TempDir::new().unwrap();
    let path = write_config(directory.path(), "general_settings: {}\n");
    let mut overlay = state(true, &Arc::new(RecordingRuntime::default()));
    let rows = database(&[("general_settings", json!({"max_parallel_requests": 4}))]);
    let source = file_source(&path);
    reload_config(&mut overlay, &source, Some(&rows), &mut BTreeMap::new())
        .await
        .unwrap();
    let reloaded = reload_config(&mut overlay, &source, None, &mut BTreeMap::new())
        .await
        .unwrap();
    assert_eq!(
        reloaded.values.into_iter().collect::<JsonValue>(),
        json!({"general_settings": {}})
    );
    assert_eq!(
        overlay.config.general_settings.get("max_parallel_requests"),
        None
    );
}

#[tokio::test]
async fn reload_includes_a_section_only_the_database_sets() {
    let directory = TempDir::new().unwrap();
    let path = write_config(
        directory.path(),
        "general_settings: {}\nrouter_settings: text\n",
    );
    let mut overlay = state(true, &Arc::new(RecordingRuntime::default()));
    let rows = database(&[("litellm_settings", json!({"num_retries": 2}))]);
    let reloaded = reload_config(
        &mut overlay,
        &file_source(&path),
        Some(&rows),
        &mut BTreeMap::new(),
    )
    .await
    .unwrap();
    assert_eq!(
        reloaded.values.into_iter().collect::<JsonValue>(),
        json!({"general_settings": {}, "router_settings": "text", "litellm_settings": {"num_retries": 2}})
    );
}

#[tokio::test]
async fn reload_merges_the_pass_through_lists_by_path() {
    let directory = TempDir::new().unwrap();
    let path = write_config(
        directory.path(),
        "general_settings:\n  pass_through_endpoints:\n    - {path: /a, target: file}\n    - {path: /b, target: file}\n",
    );
    let mut overlay = state(true, &Arc::new(RecordingRuntime::default()));
    let rows = database(&[(
        "general_settings",
        json!({"pass_through_endpoints": [{"path": "/a", "target": "db"}]}),
    )]);
    let reloaded = reload_config(
        &mut overlay,
        &file_source(&path),
        Some(&rows),
        &mut BTreeMap::new(),
    )
    .await
    .unwrap();
    assert_eq!(
        reloaded.values.into_iter().collect::<JsonValue>(),
        json!({"general_settings": {"pass_through_endpoints": [
            {"path": "/a", "target": "db"},
            {"path": "/b", "target": "file"}
        ]}})
    );
}

#[rstest]
#[case::database_not_a_list(
    json!({"pass_through_endpoints": "/a"}),
    "general_settings: {}\n",
    json!({"general_settings": {"pass_through_endpoints": "/a"}})
)]
#[case::file_list_alone(
    json!({}),
    "general_settings:\n  pass_through_endpoints: [{path: /f}]\n",
    json!({"general_settings": {"pass_through_endpoints": [{"path": "/f"}]}})
)]
#[case::file_null(
    json!({}),
    "general_settings:\n  pass_through_endpoints: null\n",
    json!({"general_settings": {}})
)]
#[tokio::test]
async fn reload_merges_pass_through_only_when_a_side_has_a_list(
    #[case] row: JsonValue,
    #[case] yaml: &str,
    #[case] expected: JsonValue,
) {
    let directory = TempDir::new().unwrap();
    let path = write_config(directory.path(), yaml);
    let mut overlay = state(true, &Arc::new(RecordingRuntime::default()));
    let rows = database(&[("general_settings", row)]);
    let reloaded = reload_config(
        &mut overlay,
        &file_source(&path),
        Some(&rows),
        &mut BTreeMap::new(),
    )
    .await
    .unwrap();
    assert_eq!(reloaded.values.into_iter().collect::<JsonValue>(), expected);
}

#[tokio::test]
async fn reload_publishes_only_the_file_and_database_pass_through_lists() {
    let directory = TempDir::new().unwrap();
    let path = write_config(directory.path(), "router_settings: {}\n");
    let mut overlay = state(false, &Arc::new(RecordingRuntime::default()));
    overlay
        .config
        .general_settings
        .insert("pass_through_endpoints", json!([{"path": "/served"}]))
        .unwrap();
    let reloaded = reload_config(
        &mut overlay,
        &file_source(&path),
        None,
        &mut BTreeMap::new(),
    )
    .await
    .unwrap();
    assert_eq!(
        reloaded.values.into_iter().collect::<JsonValue>(),
        json!({"router_settings": {}, "general_settings": {"pass_through_endpoints": []}})
    );
    assert_eq!(
        overlay
            .config
            .general_settings
            .get("pass_through_endpoints"),
        Some(&json!([]))
    );
}

#[tokio::test]
async fn reload_keeps_the_serving_list_when_its_row_goes() {
    let directory = TempDir::new().unwrap();
    let path = write_config(directory.path(), "general_settings: {}\n");
    let source = file_source(&path);
    let mut overlay = state(true, &Arc::new(RecordingRuntime::default()));
    let rows = database(&[(
        "general_settings",
        json!({"pass_through_endpoints": [{"path": "/db"}]}),
    )]);
    reload_config(&mut overlay, &source, Some(&rows), &mut BTreeMap::new())
        .await
        .unwrap();
    let reloaded = reload_config(&mut overlay, &source, None, &mut BTreeMap::new())
        .await
        .unwrap();
    assert_eq!(
        reloaded.values["general_settings"],
        json!({"pass_through_endpoints": []})
    );
}

#[tokio::test]
async fn reload_starts_the_secret_manager_the_config_names() {
    let directory = TempDir::new().unwrap();
    let path = write_config(
        directory.path(),
        "general_settings:\n  key_management_system: bogus\n",
    );
    let mut overlay = state(false, &Arc::new(RecordingRuntime::default()));
    let error = reload_config(
        &mut overlay,
        &file_source(&path),
        None,
        &mut BTreeMap::new(),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, OverlayError::SecretManager(_)));
}

#[tokio::test]
async fn reload_resolves_environment_references() {
    let yaml = "model_list:\n  - litellm_params:\n      api_key: os.environ/KEY\ngeneral_settings:\n  master_key: os.environ/MASTER\n  nested:\n    inner: os.environ/INNER\n  entries:\n    - {key: os.environ/ENTRY}\n    - os.environ/TEXT\n  missing: os.environ/ABSENT\n  flag: os.environ/FLAG\n  plain: value\n";
    let mut env = environment(&[
        ("KEY", "sk-key"),
        ("MASTER", "sk-master"),
        ("INNER", "inner"),
        ("ENTRY", "entry"),
        ("TEXT", "text"),
        ("FLAG", "True"),
    ]);
    let values = reload_file(yaml, &mut env).await.unwrap();
    assert_eq!(
        values,
        json!({
            "model_list": [{"litellm_params": {"api_key": "sk-key"}}],
            "general_settings": {
                "master_key": "sk-master",
                "nested": {"inner": "inner"},
                "entries": [{"key": "entry"}, "os.environ/TEXT"],
                "missing": null,
                "flag": true,
                "plain": "value"
            }
        })
    );
}

#[tokio::test]
async fn reload_stores_the_resolved_values_as_runtime_values() {
    let directory = TempDir::new().unwrap();
    let path = write_config(
        directory.path(),
        "litellm_settings:\n  api_base: os.environ/BASE\n",
    );
    let mut overlay = state(false, &Arc::new(RecordingRuntime::default()));
    let mut env = environment(&[("BASE", "https://base.test")]);
    reload_config(&mut overlay, &file_source(&path), None, &mut env)
        .await
        .unwrap();
    assert_eq!(
        overlay.config.litellm_settings.get("api_base"),
        Some(&json!("https://base.test"))
    );
}

#[tokio::test]
async fn reload_exports_database_environment_values_before_resolving() {
    let directory = TempDir::new().unwrap();
    let path = write_config(
        directory.path(),
        "general_settings:\n  master_key: os.environ/DB_MASTER\n",
    );
    let mut overlay = state(true, &Arc::new(RecordingRuntime::default()));
    let rows = database(&[("environment_variables", json!({"DB_MASTER": "sk-db"}))]);
    let mut env = BTreeMap::new();
    let reloaded = reload_config(&mut overlay, &file_source(&path), Some(&rows), &mut env)
        .await
        .unwrap();
    assert_eq!(
        reloaded.values["general_settings"],
        json!({"master_key": "sk-db"})
    );
    assert_eq!(env.get("DB_MASTER").map(String::as_str), Some("sk-db"));
}

// a leaf behind the given number of single-key mappings
fn nested(levels: usize, leaf: &JsonValue) -> JsonValue {
    let text = format!("{}{leaf}{}", "{\"a\":".repeat(levels), "}".repeat(levels));
    serde_json::from_str(&text).unwrap()
}

#[rstest]
#[case::at_the_limit(100, json!("deep"), false)]
#[case::past_the_limit(101, json!("os.environ/DEEP"), true)]
fn resolver_stops_past_the_depth_limit(
    #[case] levels: usize,
    #[case] leaf: JsonValue,
    #[case] warned: bool,
) {
    let (logs, _guard) = capture_logs();
    let lookup = secret_lookup(None);
    let env = environment(&[("DEEP", "deep")]);
    let resolver = Resolver {
        lookup: &lookup,
        env: &env,
    };
    let mut value = nested(levels, &json!("os.environ/DEEP"));
    resolver.resolve_entries([&mut value], 0).unwrap();
    assert_eq!(value, nested(levels, &leaf));
    assert_eq!(
        logs.contents()
            .contains("Maximum recursion depth (100) reached while processing config."),
        warned
    );
}

#[test]
fn resolver_leaves_values_without_a_reference_alone() {
    let lookup = secret_lookup(None);
    let env = BTreeMap::new();
    let resolver = Resolver {
        lookup: &lookup,
        env: &env,
    };
    let mut value = json!({"a": "environ/A", "b": 1, "c": null, "d": false, "e": "x os.environ/A"});
    let before = value.clone();
    resolver.resolve_entries([&mut value], 0).unwrap();
    assert_eq!(value, before);
}
