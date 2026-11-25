use std::error::Error as _;

use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::stored_cipher::DbCipher;

use super::support::{
    ControlLog, FakeReconnect, FakeSso, ReconnectResult, SsoReply, capture_logs, cipher, env_value,
    reloader, reloader_with, sealed,
};
use crate::db_settings_reload::SettingsReloadError;
use crate::db_settings_reload::ports::NoReconnect;
use crate::db_settings_reload::sso::read_sso_row;

const SSO_ERROR: &str = "litellm.proxy.proxy_server.py::ProxyConfig:_init_sso_settings_in_db - ";

fn sso_row(cipher: &DbCipher, extra: &str) -> String {
    format!(
        "{{\"role_mappings\":\"{}\",\"team_mappings\":\"{}\",\"ui_access_mode\":\"{}\",\"google_client_id\":\"{}\",{extra}}}",
        sealed(cipher, "roles"),
        sealed(cipher, "teams"),
        sealed(cipher, "redirect"),
        sealed(cipher, "google-id"),
    )
}

#[tokio::test]
async fn a_row_is_upper_cased_and_its_decrypted_values_exported() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let cipher = cipher();
    let row = sso_row(
        &cipher,
        &format!(
            "\"GENERIC_CLIENT_SECRET\":\"{}\"",
            sealed(&cipher, "secret")
        ),
    );
    let db = FakeSso::new(SsoReply::Row(Some(row)));
    let (logs, _guard) = capture_logs();
    reloader.reload_sso(&db).await;
    assert_eq!(
        env_value(&reloader, "GOOGLE_CLIENT_ID"),
        Some("google-id".to_owned())
    );
    assert_eq!(
        env_value(&reloader, "GENERIC_CLIENT_SECRET"),
        Some("secret".to_owned())
    );
    assert_eq!(env_value(&reloader, "ROLE_MAPPINGS"), None);
    assert_eq!(env_value(&reloader, "TEAM_MAPPINGS"), None);
    assert_eq!(env_value(&reloader, "UI_ACCESS_MODE"), None);
    assert_eq!(logs.contents(), "");
}

#[tokio::test]
async fn an_empty_value_is_exported_as_an_empty_string() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let cipher = cipher();
    let row = format!("\"EMPTY_SETTING\":\"{}\"", sealed(&cipher, ""));
    let db = FakeSso::new(SsoReply::Row(Some(format!("{{{row}}}"))));
    let (_logs, _guard) = capture_logs();
    reloader.reload_sso(&db).await;
    assert_eq!(env_value(&reloader, "EMPTY_SETTING"), Some(String::new()));
}

#[tokio::test]
async fn a_value_that_does_not_decrypt_is_not_exported() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let db = FakeSso::new(SsoReply::Row(Some(
        "{\"plain_key\":\"not-a-ciphertext\"}".to_owned(),
    )));
    let (_logs, _guard) = capture_logs();
    reloader.reload_sso(&db).await;
    assert_eq!(env_value(&reloader, "PLAIN_KEY"), None);
}

#[tokio::test]
async fn a_missing_row_sets_no_environment() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let db = FakeSso::new(SsoReply::Row(None));
    let (logs, _guard) = capture_logs();
    reloader.reload_sso(&db).await;
    assert!(reloader.environment.snapshot().values.is_empty());
    assert_eq!(logs.contents(), "");
}

#[tokio::test]
async fn a_row_that_is_not_an_object_is_logged() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let db = FakeSso::new(SsoReply::Row(Some("[]".to_owned())));
    let (logs, _guard) = capture_logs();
    reloader.reload_sso(&db).await;
    assert!(
        logs.contents()
            .contains("sso_settings is not a JSON object")
    );
    assert!(reloader.environment.snapshot().values.is_empty());
}

#[tokio::test]
async fn invalid_json_is_logged() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let db = FakeSso::new(SsoReply::Row(Some("not json".to_owned())));
    let (logs, _guard) = capture_logs();
    reloader.reload_sso(&db).await;
    assert!(
        logs.contents()
            .contains("a stored settings value is not valid JSON")
    );
}

#[tokio::test]
async fn a_transport_failure_retries_with_the_sso_reason() {
    let control = ControlLog::default();
    let (reconnect, reasons) = FakeReconnect::new(ReconnectResult::Reconnected);
    let mut reloader = reloader_with(&control, Some(reconnect));
    let cipher = cipher();
    let row = format!(
        "{{\"google_client_id\":\"{}\"}}",
        sealed(&cipher, "google-id")
    );
    let db = FakeSso::new(SsoReply::Flaky(row));
    let (_logs, _guard) = capture_logs();
    reloader.reload_sso(&db).await;
    assert_eq!(db.calls(), 2);
    assert_eq!(
        *reasons.lock().unwrap(),
        vec!["init_sso_settings_in_db_lookup_failure".to_owned()]
    );
    assert_eq!(
        env_value(&reloader, "GOOGLE_CLIENT_ID"),
        Some("google-id".to_owned())
    );
}

#[tokio::test]
async fn a_transport_failure_without_a_reconnect_handle_is_logged() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let db = FakeSso::new(SsoReply::Transport);
    let (logs, _guard) = capture_logs();
    reloader.reload_sso(&db).await;
    assert_eq!(db.calls(), 1);
    assert!(logs.contents().contains(SSO_ERROR));
    assert!(reloader.environment.snapshot().values.is_empty());
}

#[tokio::test]
async fn a_declined_reconnect_keeps_the_error() {
    let control = ControlLog::default();
    let (reconnect, reasons) = FakeReconnect::new(ReconnectResult::Declined);
    let mut reloader = reloader_with(&control, Some(reconnect));
    let db = FakeSso::new(SsoReply::Transport);
    let (logs, _guard) = capture_logs();
    reloader.reload_sso(&db).await;
    assert_eq!(db.calls(), 1);
    assert_eq!(reasons.lock().unwrap().len(), 1);
    assert!(logs.contents().contains(SSO_ERROR));
}

#[tokio::test]
async fn a_reconnect_error_is_logged() {
    let control = ControlLog::default();
    let (reconnect, _reasons) = FakeReconnect::new(ReconnectResult::Failed);
    let mut reloader = reloader_with(&control, Some(reconnect));
    let db = FakeSso::new(SsoReply::Transport);
    let (logs, _guard) = capture_logs();
    reloader.reload_sso(&db).await;
    assert_eq!(db.calls(), 1);
    assert!(logs.contents().contains(SSO_ERROR));
}

#[tokio::test]
async fn a_row_read_failure_keeps_the_table_error_as_its_source() {
    let db = FakeSso::new(SsoReply::Failed);
    let mut reconnect: Option<NoReconnect> = None;
    let error = read_sso_row(reconnect.as_mut(), &db).await.unwrap_err();
    assert!(matches!(
        &error,
        SettingsReloadError::SsoRow(TableError::MissingRow)
    ));
    let source = error.source().unwrap();
    assert_eq!(source.to_string(), "statement returned no row");
}

#[tokio::test]
async fn a_failure_that_is_not_a_transport_error_is_not_retried() {
    let control = ControlLog::default();
    let (reconnect, reasons) = FakeReconnect::new(ReconnectResult::Reconnected);
    let mut reloader = reloader_with(&control, Some(reconnect));
    let db = FakeSso::new(SsoReply::Failed);
    let (logs, _guard) = capture_logs();
    reloader.reload_sso(&db).await;
    assert_eq!(db.calls(), 1);
    assert!(reasons.lock().unwrap().is_empty());
    assert!(logs.contents().contains(SSO_ERROR));
}
