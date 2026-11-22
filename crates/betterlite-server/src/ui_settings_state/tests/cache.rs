use betterlite_config::sections::settings::settings_resolution::FieldSource;
use betterlite_config::sections::settings::settings_rules::Section;
use betterlite_config::sections::settings::settings_store::SettingsStore;
use betterlite_config::ui_settings::UiSettingsDict;
use betterlite_models::decisions::Json;

use super::support::{FakeRepository, Harness, cache_options, json, settings, stored};
use crate::ui_settings_state::cache::{
    UI_SETTINGS_CACHE_KEY, UiSettingsReadError, get_ui_settings_cached, sync_ui_settings,
};
use crate::ui_settings_state::decode::{StoredUiSettingsError, decode_stored_ui_settings};
use crate::ui_settings_state::projection::{GeneralSettingsTarget, apply_runtime_ui_flags};
use crate::ui_settings_state::values::decision_object;

const ROW: &str = r#"{
    "allow_public_health_readiness_details": true,
    "forward_client_headers_to_llm_api": true,
    "forward_llm_provider_auth_headers": true,
    "disable_agents_for_internal_users": true,
    "allow_agents_for_team_admins": true,
    "disable_vector_stores_for_internal_users": true,
    "allow_vector_stores_for_team_admins": true,
    "disable_custom_api_keys": true,
    "disable_key_generate_for_org_admin": true,
    "team_admin_editable_team_fields": ["max_budget"],
    "enable_chat_ui": true,
    "scope_user_search_to_org": true,
    "require_auth_for_public_ai_hub": true,
    "not_a_ui_setting": 1
}"#;

const RUNTIME_FLAGS: &str = r#"{
    "allow_public_health_readiness_details": true,
    "forward_client_headers_to_llm_api": true,
    "forward_llm_provider_auth_headers": true,
    "disable_agents_for_internal_users": true,
    "allow_agents_for_team_admins": true,
    "disable_vector_stores_for_internal_users": true,
    "allow_vector_stores_for_team_admins": true,
    "disable_custom_api_keys": true,
    "disable_key_generate_for_org_admin": true,
    "team_admin_editable_team_fields": ["max_budget"]
}"#;

const SANITIZED_ROW: &str = r#"{
    "allow_public_health_readiness_details": true,
    "forward_client_headers_to_llm_api": true,
    "forward_llm_provider_auth_headers": true,
    "disable_agents_for_internal_users": true,
    "allow_agents_for_team_admins": true,
    "disable_vector_stores_for_internal_users": true,
    "allow_vector_stores_for_team_admins": true,
    "disable_custom_api_keys": true,
    "disable_key_generate_for_org_admin": true,
    "team_admin_editable_team_fields": ["max_budget"],
    "require_auth_for_public_ai_hub": true,
    "scope_user_search_to_org": true,
    "enable_chat_ui": true
}"#;

async fn reads_after_advancing(seconds: u32) -> usize {
    let harness = Harness::new();
    let repository = FakeRepository::new(Some(stored(r#"{"enable_chat_ui": true}"#)));
    let first = get_ui_settings_cached(&harness.dual, Some(&repository))
        .await
        .expect("the settings load");
    assert_eq!(first, settings(r#"{"enable_chat_ui": true}"#));
    harness.advance(seconds);
    let second = get_ui_settings_cached(&harness.dual, Some(&repository))
        .await
        .expect("the settings load");
    assert_eq!(second, settings(r#"{"enable_chat_ui": true}"#));
    repository.reads()
}

#[tokio::test]
async fn ttl_serves_the_cached_dictionary_inside_ten_minutes() {
    assert_eq!(reads_after_advancing(599).await, 1);
    assert_eq!(reads_after_advancing(600).await, 1);
}

#[tokio::test]
async fn ttl_reads_the_row_again_after_ten_minutes() {
    assert_eq!(reads_after_advancing(601).await, 2);
}

#[tokio::test]
async fn cache_serves_a_dictionary_unchanged() {
    let harness = Harness::new();
    let repository = FakeRepository::new(Some(stored(r#"{"enable_chat_ui": false}"#)));
    let cached = decision_object(&settings(
        r#"{"enable_chat_ui": true, "not_a_ui_setting": {"nested": [1]}}"#,
    ));
    harness
        .dual
        .async_set_cache(UI_SETTINGS_CACHE_KEY, &cached, cache_options())
        .await;
    let loaded = get_ui_settings_cached(&harness.dual, Some(&repository))
        .await
        .expect("the cache serves");
    assert_eq!(
        loaded,
        settings(r#"{"enable_chat_ui": true, "not_a_ui_setting": {"nested": [1]}}"#)
    );
    assert_eq!(repository.reads(), 0);
}

#[tokio::test]
async fn cache_ignores_a_value_that_is_not_a_dictionary() {
    let harness = Harness::new();
    let repository = FakeRepository::new(Some(stored(r#"{"enable_chat_ui": true}"#)));
    harness
        .dual
        .async_set_cache(
            UI_SETTINGS_CACHE_KEY,
            &Json::Text("stale".to_owned()),
            cache_options(),
        )
        .await;
    let loaded = get_ui_settings_cached(&harness.dual, Some(&repository))
        .await
        .expect("the settings load");
    assert_eq!(loaded, settings(r#"{"enable_chat_ui": true}"#));
    assert_eq!(repository.reads(), 1);
}

#[tokio::test]
async fn cache_sanitizes_a_database_read_and_serves_it_afterwards() {
    let harness = Harness::new();
    let repository = FakeRepository::new(Some(stored(ROW)));
    let loaded = get_ui_settings_cached(&harness.dual, Some(&repository))
        .await
        .expect("the settings load");
    assert_eq!(loaded, settings(SANITIZED_ROW));
    let again = get_ui_settings_cached(&harness.dual, Some(&repository))
        .await
        .expect("the cache serves");
    assert_eq!(again, loaded);
    assert_eq!(repository.reads(), 1);
}

#[tokio::test]
async fn cache_stores_an_empty_dictionary_when_the_row_is_absent() {
    let harness = Harness::new();
    let repository = FakeRepository::new(None);
    let loaded = get_ui_settings_cached(&harness.dual, Some(&repository))
        .await
        .expect("the settings load");
    assert_eq!(loaded, UiSettingsDict::new());
    assert_eq!(repository.reads(), 1);
    let again = get_ui_settings_cached(&harness.dual, Some(&repository))
        .await
        .expect("the cache serves");
    assert_eq!(again, UiSettingsDict::new());
    assert_eq!(repository.reads(), 1);
}

#[tokio::test]
async fn cache_returns_empty_without_a_database_and_stores_nothing() {
    let harness = Harness::new();
    let loaded = get_ui_settings_cached(&harness.dual, None)
        .await
        .expect("the settings load");
    assert_eq!(loaded, UiSettingsDict::new());
    let repository = FakeRepository::new(Some(stored(r#"{"enable_chat_ui": true}"#)));
    let served = get_ui_settings_cached(&harness.dual, Some(&repository))
        .await
        .expect("the settings load");
    assert_eq!(served, settings(r#"{"enable_chat_ui": true}"#));
    assert_eq!(repository.reads(), 1);
}

#[tokio::test]
async fn cache_propagates_malformed_stored_json_and_stores_nothing() {
    let harness = Harness::new();
    let repository = FakeRepository::new(Some(stored(r#""not json""#)));
    let error = get_ui_settings_cached(&harness.dual, Some(&repository))
        .await
        .expect_err("the row is malformed");
    assert!(matches!(
        error,
        UiSettingsReadError::Stored(StoredUiSettingsError::Json(_))
    ));
    assert!(
        harness
            .dual
            .async_get_cache(UI_SETTINGS_CACHE_KEY, false)
            .await
            .is_none()
    );
}

#[tokio::test]
async fn cache_accepts_a_json_string_row() {
    let harness = Harness::new();
    let encoded = serde_json::to_string(r#"{"enable_chat_ui": true}"#).expect("the row serializes");
    let repository = FakeRepository::new(Some(stored(&encoded)));
    let loaded = get_ui_settings_cached(&harness.dual, Some(&repository))
        .await
        .expect("the settings load");
    assert_eq!(loaded, settings(r#"{"enable_chat_ui": true}"#));
}

#[tokio::test]
async fn sync_applies_only_the_ten_runtime_flags() {
    let repository = FakeRepository::new(Some(stored(ROW)));
    let mut store = SettingsStore::from(Section::GeneralSettings);
    let applied = sync_ui_settings(&repository, GeneralSettingsTarget::Store(&mut store)).await;
    assert_eq!(applied, settings(RUNTIME_FLAGS));
    assert_eq!(
        store.get("forward_client_headers_to_llm_api"),
        Some(&json("true"))
    );
    assert_eq!(
        store
            .resolution("forward_client_headers_to_llm_api")
            .source(),
        FieldSource::Db
    );
    assert!(store.get("enable_chat_ui").is_none());
    assert!(store.get("scope_user_search_to_org").is_none());
    assert!(store.get("require_auth_for_public_ai_hub").is_none());
    assert!(store.get("not_a_ui_setting").is_none());
}

#[tokio::test]
async fn sync_clears_the_flags_when_the_row_is_removed() {
    let repository = FakeRepository::new(Some(stored(
        r#"{"forward_client_headers_to_llm_api": true}"#,
    )));
    let mut store = SettingsStore::from(Section::GeneralSettings);
    drop(sync_ui_settings(&repository, GeneralSettingsTarget::Store(&mut store)).await);
    assert_eq!(
        store.get("forward_client_headers_to_llm_api"),
        Some(&json("true"))
    );
    repository.remove_row();
    let applied = sync_ui_settings(&repository, GeneralSettingsTarget::Store(&mut store)).await;
    assert_eq!(applied, UiSettingsDict::new());
    assert!(store.get("forward_client_headers_to_llm_api").is_none());
}

#[tokio::test]
async fn sync_updates_only_present_entries_in_a_plain_map() {
    let repository = FakeRepository::new(Some(stored(
        r#"{"allow_public_health_readiness_details": true}"#,
    )));
    let mut map = settings(r#"{"forward_client_headers_to_llm_api": false, "unrelated": 1}"#);
    let applied = sync_ui_settings(&repository, GeneralSettingsTarget::Map(&mut map)).await;
    assert_eq!(
        applied,
        settings(r#"{"allow_public_health_readiness_details": true}"#)
    );
    assert_eq!(
        map,
        settings(
            r#"{"allow_public_health_readiness_details": true, "forward_client_headers_to_llm_api": false, "unrelated": 1}"#
        )
    );
    repository.remove_row();
    let removed = sync_ui_settings(&repository, GeneralSettingsTarget::Map(&mut map)).await;
    assert_eq!(removed, UiSettingsDict::new());
    assert_eq!(
        map,
        settings(
            r#"{"allow_public_health_readiness_details": true, "forward_client_headers_to_llm_api": false, "unrelated": 1}"#
        )
    );
}

#[tokio::test]
async fn sync_keeps_the_current_flags_on_a_read_error() {
    let repository = FakeRepository::new(Some(stored(
        r#"{"forward_client_headers_to_llm_api": true}"#,
    )));
    let mut store = SettingsStore::from(Section::GeneralSettings);
    drop(sync_ui_settings(&repository, GeneralSettingsTarget::Store(&mut store)).await);
    repository.deny_reads();
    let applied = sync_ui_settings(&repository, GeneralSettingsTarget::Store(&mut store)).await;
    assert_eq!(applied, UiSettingsDict::new());
    assert_eq!(
        store.get("forward_client_headers_to_llm_api"),
        Some(&json("true"))
    );
}

#[tokio::test]
async fn sync_keeps_the_current_flags_on_a_decode_error() {
    let repository = FakeRepository::new(Some(stored(
        r#"{"forward_client_headers_to_llm_api": true}"#,
    )));
    let mut store = SettingsStore::from(Section::GeneralSettings);
    drop(sync_ui_settings(&repository, GeneralSettingsTarget::Store(&mut store)).await);
    let broken = FakeRepository::new(Some(stored(r#""not json""#)));
    let applied = sync_ui_settings(&broken, GeneralSettingsTarget::Store(&mut store)).await;
    assert_eq!(applied, UiSettingsDict::new());
    assert_eq!(
        store.get("forward_client_headers_to_llm_api"),
        Some(&json("true"))
    );
}

#[tokio::test]
async fn sync_does_not_refresh_the_dual_cache() {
    let harness = Harness::new();
    let cached = decision_object(&settings(r#"{"forward_client_headers_to_llm_api": false}"#));
    harness
        .dual
        .async_set_cache(UI_SETTINGS_CACHE_KEY, &cached, cache_options())
        .await;
    let repository = FakeRepository::new(Some(stored(
        r#"{"forward_client_headers_to_llm_api": true}"#,
    )));
    let mut map = UiSettingsDict::new();
    drop(sync_ui_settings(&repository, GeneralSettingsTarget::Map(&mut map)).await);
    assert_eq!(
        harness
            .dual
            .async_get_cache(UI_SETTINGS_CACHE_KEY, false)
            .await,
        Some(cached)
    );
}

#[tokio::test]
async fn sync_reads_the_repository_directly() {
    let harness = Harness::new();
    let stale = decision_object(&settings(r#"{"forward_client_headers_to_llm_api": false}"#));
    harness
        .dual
        .async_set_cache(UI_SETTINGS_CACHE_KEY, &stale, cache_options())
        .await;
    let repository = FakeRepository::new(Some(stored(
        r#"{"forward_client_headers_to_llm_api": true}"#,
    )));
    let mut store = SettingsStore::from(Section::GeneralSettings);
    let applied = sync_ui_settings(&repository, GeneralSettingsTarget::Store(&mut store)).await;
    assert_eq!(
        applied,
        settings(r#"{"forward_client_headers_to_llm_api": true}"#)
    );
    assert_eq!(repository.reads(), 1);
}

#[test]
fn projection_returns_only_the_runtime_fields_present() {
    let row = settings(r#"{"enable_chat_ui": true, "forward_client_headers_to_llm_api": false}"#);
    let mut map = UiSettingsDict::new();
    let applied = apply_runtime_ui_flags(&row, GeneralSettingsTarget::Map(&mut map));
    assert_eq!(
        applied,
        settings(r#"{"forward_client_headers_to_llm_api": false}"#)
    );
    assert_eq!(map, applied);
}

#[test]
fn projection_replaces_the_stored_row_so_removed_fields_clear() {
    let mut store = SettingsStore::from(Section::GeneralSettings);
    drop(apply_runtime_ui_flags(
        &settings(r#"{"forward_client_headers_to_llm_api": true, "enable_chat_ui": true}"#),
        GeneralSettingsTarget::Store(&mut store),
    ));
    assert_eq!(
        store.get("forward_client_headers_to_llm_api"),
        Some(&json("true"))
    );
    drop(apply_runtime_ui_flags(
        &settings(r#"{"enable_chat_ui": false}"#),
        GeneralSettingsTarget::Store(&mut store),
    ));
    assert_eq!(store.get("forward_client_headers_to_llm_api"), None);
}

#[test]
fn decode_accepts_an_object_a_string_object_and_null() {
    assert_eq!(
        decode_stored_ui_settings(br#"{"enable_chat_ui": true}"#).expect("the row decodes"),
        settings(r#"{"enable_chat_ui": true}"#)
    );
    let encoded = serde_json::to_vec(r#"{"enable_chat_ui": true}"#).expect("the row serializes");
    assert_eq!(
        decode_stored_ui_settings(&encoded).expect("the row decodes"),
        settings(r#"{"enable_chat_ui": true}"#)
    );
    assert_eq!(
        decode_stored_ui_settings(b"null").expect("null is empty"),
        UiSettingsDict::new()
    );
    assert_eq!(
        decode_stored_ui_settings(br#"{"nested": {"a": [1, null]}}"#).expect("the row decodes"),
        settings(r#"{"nested": {"a": [1, null]}}"#)
    );
}

#[test]
fn decode_rejects_rows_that_are_not_objects() {
    assert!(matches!(
        decode_stored_ui_settings(b"[1]"),
        Err(StoredUiSettingsError::NotAnObject)
    ));
    assert!(matches!(
        decode_stored_ui_settings(b"5"),
        Err(StoredUiSettingsError::NotAnObject)
    ));
    assert!(matches!(
        decode_stored_ui_settings(b"true"),
        Err(StoredUiSettingsError::NotAnObject)
    ));
    let array_text = serde_json::to_vec("[1]").expect("the row serializes");
    assert!(matches!(
        decode_stored_ui_settings(&array_text),
        Err(StoredUiSettingsError::NotAnObject)
    ));
    let null_text = serde_json::to_vec("null").expect("the row serializes");
    assert!(matches!(
        decode_stored_ui_settings(&null_text),
        Err(StoredUiSettingsError::NotAnObject)
    ));
}

#[test]
fn decode_rejects_malformed_json() {
    assert!(matches!(
        decode_stored_ui_settings(b"{"),
        Err(StoredUiSettingsError::Json(_))
    ));
    let empty_text = serde_json::to_vec("").expect("the row serializes");
    assert!(matches!(
        decode_stored_ui_settings(&empty_text),
        Err(StoredUiSettingsError::Json(_))
    ));
    let invalid_utf8 = decode_stored_ui_settings(&[0xff, 0xfe]);
    assert!(matches!(invalid_utf8, Err(StoredUiSettingsError::Json(_))));
}
