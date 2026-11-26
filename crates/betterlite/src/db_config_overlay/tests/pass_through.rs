use betterlite_config::sections::settings::JsonValue;
use rstest::rstest;

use std::sync::PoisonError;

use super::support::{Fixture, fixture, json};
use crate::db_config_overlay::error::OverlayError;
use crate::db_config_overlay::pass_through::endpoints_beside_db;
use crate::db_config_overlay::state::OverlayState;

#[rstest]
#[case::database_path_wins(
    json!([{"path": "/b", "target": "db"}]),
    json!([{"path": "/a", "target": "file"}, {"path": "/b", "target": "file"}]),
    json!([{"path": "/b", "target": "db"}, {"path": "/a", "target": "file"}])
)]
#[case::pathless_entries_share_a_path(
    json!([{"target": "db"}]),
    json!([{"target": "file"}, {"path": null, "target": "file"}]),
    json!([{"target": "db"}])
)]
#[case::file_text_entries_stay(
    json!(["db-text"]),
    json!(["file-text", {"path": "/a"}]),
    json!(["db-text", "file-text", {"path": "/a"}])
)]
#[case::empty_database(
    json!([]),
    json!([{"path": "/a"}]),
    json!([{"path": "/a"}])
)]
fn pass_through_effect_merges_by_path(
    #[case] database: JsonValue,
    #[case] file: JsonValue,
    #[case] merged: JsonValue,
) {
    let database_entries = database.as_array().unwrap();
    let file_entries = file.as_array().unwrap();
    assert_eq!(
        JsonValue::Array(endpoints_beside_db(database_entries, file_entries)),
        merged
    );
}

#[tokio::test]
async fn pass_through_effect_installs_the_database_list_in_stored_order() {
    let mut fixture = fixture();
    let mut state = fixture.state();
    state.startup_pass_through = vec![
        json!({"path": "/file", "target": "https://file.test"}),
        json!({"path": "/b", "target": "https://file-b.test"}),
    ];
    let row = json!({"pass_through_endpoints": [
        {"path": "/b", "target": "https://db-b.test"},
        {"path": "/a", "target": "https://db-a.test"}
    ]});
    fixture.apply(&mut state, &row).await.unwrap();
    assert_eq!(
        fixture.installer.calls(),
        [vec![
            json!({"path": "/b", "target": "https://db-b.test"}),
            json!({"path": "/a", "target": "https://db-a.test"})
        ]]
    );
    assert_eq!(
        state.config.general_settings.get("pass_through_endpoints"),
        Some(&json!([
            {"path": "/b", "target": "https://db-b.test"},
            {"path": "/a", "target": "https://db-a.test"},
            {"path": "/file", "target": "https://file.test"}
        ]))
    );
}

#[tokio::test]
async fn pass_through_effect_publishes_the_startup_list_when_the_store_has_none() {
    let mut fixture = fixture();
    let mut state = fixture.state();
    state.startup_pass_through = vec![json!({"path": "/file"})];
    fixture.apply(&mut state, &json!({})).await.unwrap();
    assert!(fixture.installer.calls().is_empty());
    assert_eq!(
        state.config.general_settings.get("pass_through_endpoints"),
        Some(&json!([{"path": "/file"}]))
    );
}

#[tokio::test]
async fn pass_through_effect_stores_a_value_that_is_not_a_list_without_serving_it() {
    let mut fixture = fixture();
    let mut state = fixture.state();
    state.startup_pass_through = vec![json!({"path": "/file"})];
    let row = json!({"pass_through_endpoints": "/a"});
    fixture.apply(&mut state, &row).await.unwrap();
    assert!(fixture.installer.calls().is_empty());
    assert_eq!(
        state.config.general_settings.get("pass_through_endpoints"),
        Some(&json!("/a"))
    );
}

#[tokio::test]
async fn pass_through_effect_keeps_a_served_list_when_the_row_has_none() {
    let mut fixture = fixture();
    let mut state = fixture.state();
    state
        .config
        .general_settings
        .insert("pass_through_endpoints", json!([{"path": "/served"}]))
        .unwrap();
    state.startup_pass_through = vec![json!({"path": "/file"})];
    fixture
        .apply(&mut state, &json!({"master_key": "sk-db"}))
        .await
        .unwrap();
    assert!(fixture.installer.calls().is_empty());
    assert_eq!(
        state.config.general_settings.get("pass_through_endpoints"),
        Some(&json!([{"path": "/served"}]))
    );
}

#[tokio::test]
async fn pass_through_effect_refuses_a_non_object_entry_and_stops_the_effects() {
    let mut fixture = fixture();
    let mut state = fixture.state();
    state.config.store_model_in_db = false;
    let row = json!({
        "pass_through_endpoints": [{"path": "/a"}, "/b"],
        "store_prompts_in_spend_logs": "true",
        "user_api_key_cache_max_size": 5,
        "store_model_in_db": true,
        "maximum_spend_logs_retention_period": "1d",
        "user_url_validation": true
    });
    let error = fixture.apply(&mut state, &row).await.unwrap_err();
    assert!(matches!(error, OverlayError::PassThroughEntry { index: 1 }));
    assert!(fixture.installer.calls().is_empty());
    assert_eq!(
        state.config.general_settings.get("pass_through_endpoints"),
        Some(&json!([{"path": "/a"}, "/b"]))
    );
    assert_stopped(&fixture, &state);
}

#[tokio::test]
async fn pass_through_effect_install_error_stops_the_effects() {
    let mut fixture = fixture();
    fixture.installer.fails = true;
    let mut state = fixture.state();
    state.config.store_model_in_db = false;
    let row = json!({
        "pass_through_endpoints": [{"path": "/a"}],
        "store_prompts_in_spend_logs": "true",
        "user_api_key_cache_max_size": 5,
        "store_model_in_db": true,
        "maximum_spend_logs_retention_period": "1d",
        "user_url_validation": true
    });
    let error = fixture.apply(&mut state, &row).await.unwrap_err();
    assert!(matches!(error, OverlayError::PassThroughInstall { .. }));
    assert_eq!(fixture.installer.calls(), [vec![json!({"path": "/a"})]]);
    assert_eq!(
        state.config.general_settings.get("pass_through_endpoints"),
        Some(&json!([{"path": "/a"}]))
    );
    assert_stopped(&fixture, &state);
}

fn assert_stopped(fixture: &Fixture, state: &OverlayState) {
    let store = &state.config.general_settings;
    assert_eq!(
        store.get("store_prompts_in_spend_logs"),
        Some(&json!("true"))
    );
    assert_eq!(store.get("user_api_key_cache_max_size"), Some(&json!(5)));
    assert!(!state.config.store_model_in_db);
    assert!(fixture.scheduler.inserted.is_empty());
    assert!(fixture.runtime.effects().is_empty());
    assert!(
        fixture
            .cache
            .sizes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    );
}
