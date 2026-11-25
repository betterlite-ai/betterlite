use betterlite_config::sections::settings::JsonValue;
use betterlite_db::database_faults::{DbError, ErrorType};
use betterlite_db::stored_cipher::DbCipher;
use betterlite_secret_managers::secret_types::KeyManagementSystem;
use rstest::rstest;

use super::super::SettingsReloader;
use super::support::{
    ControlLog, FakeReconnect, FakeRows, ReconnectResult, capture_logs, cipher, env_value, fault,
    json, reloader, reloader_with, sealed,
};

const VAULT_ERROR: &str = "Error loading Hashicorp Vault config override from DB: ";
const CYBERARK_ERROR: &str = "Error loading CyberArk config override from DB: ";

fn vault_rows(config: JsonValue) -> FakeRows {
    FakeRows {
        vault: Some(config),
        ..FakeRows::default()
    }
}

fn cyberark_rows(config: JsonValue) -> FakeRows {
    FakeRows {
        cyberark: Some(config),
        ..FakeRows::default()
    }
}

fn vault_config(cipher: &DbCipher, addr: &str, token: &str) -> String {
    format!(
        "{{\"vault_addr\":\"{addr}\",\"vault_token\":\"{}\"}}",
        sealed(cipher, token)
    )
}

fn cyberark_config(cipher: &DbCipher, api_base: &str, api_key: &str) -> String {
    format!(
        "{{\"cyberark_api_base\":\"{api_base}\",\"cyberark_api_key\":\"{}\"}}",
        sealed(cipher, api_key)
    )
}

#[tokio::test]
async fn a_vault_row_sets_the_mapped_environment_and_initializes_the_manager() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let cipher = cipher();
    let config = format!(
        "{{\"vault_addr\":\"addr\",\"vault_token\":\"{}\",\"vault_namespace\":\"ns\"}}",
        sealed(&cipher, "token")
    );
    let (logs, _guard) = capture_logs();
    reloader.reload_vault(&vault_rows(json(&config))).await;
    assert_eq!(
        env_value(&reloader, "HCP_VAULT_ADDR"),
        Some("addr".to_owned())
    );
    assert_eq!(
        env_value(&reloader, "HCP_VAULT_TOKEN"),
        Some("token".to_owned())
    );
    assert_eq!(
        env_value(&reloader, "HCP_VAULT_NAMESPACE"),
        Some("ns".to_owned())
    );
    assert_eq!(
        control.initialized(),
        vec![KeyManagementSystem::HashicorpVault]
    );
    assert_eq!(reloader.state.vault_last_config, Some(json(&config)));
    assert_eq!(reloader.state.cyberark_boot_env, None);
    assert!(
        logs.contents()
            .contains("Hashicorp Vault config override loaded from DB")
    );
}

#[tokio::test]
async fn a_vault_string_row_is_decoded() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let row = JsonValue::String("{\"vault_addr\":\"addr\"}".to_owned());
    let (_logs, _guard) = capture_logs();
    reloader.reload_vault(&vault_rows(row)).await;
    assert_eq!(
        env_value(&reloader, "HCP_VAULT_ADDR"),
        Some("addr".to_owned())
    );
}

#[tokio::test]
async fn a_vault_value_that_cannot_be_decrypted_keeps_its_stored_text() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let row = json("{\"vault_token\":\"not-a-ciphertext\"}");
    let (_logs, _guard) = capture_logs();
    reloader.reload_vault(&vault_rows(row)).await;
    assert_eq!(
        env_value(&reloader, "HCP_VAULT_TOKEN"),
        Some("not-a-ciphertext".to_owned())
    );
}

#[tokio::test]
async fn a_mapped_variable_missing_from_the_row_is_cleared() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    reloader
        .environment
        .boot
        .insert("HCP_VAULT_PATH_PREFIX".to_owned(), "boot-prefix".to_owned());
    let (_logs, _guard) = capture_logs();
    reloader
        .reload_vault(&vault_rows(json("{\"vault_addr\":\"addr\"}")))
        .await;
    assert_eq!(env_value(&reloader, "HCP_VAULT_PATH_PREFIX"), None);
}

#[tokio::test]
async fn an_unchanged_vault_row_does_nothing() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let rows = vault_rows(json("{\"vault_addr\":\"addr\"}"));
    let (_logs, _guard) = capture_logs();
    reloader.reload_vault(&rows).await;
    control.reset();
    let before = reloader.state.vault_last_config.clone();
    reloader.reload_vault(&rows).await;
    assert!(control.initialized().is_empty());
    assert_eq!(reloader.state.vault_last_config, before);
    assert_eq!(
        env_value(&reloader, "HCP_VAULT_ADDR"),
        Some("addr".to_owned())
    );
}

#[tokio::test]
async fn a_reencrypted_value_counts_as_a_change() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let cipher = cipher();
    let first = format!("\"vault_token\":\"{}\"", sealed(&cipher, "token"));
    let second = format!("\"vault_token\":\"{}\"", sealed(&cipher, "token"));
    let (_logs, _guard) = capture_logs();
    reloader
        .reload_vault(&vault_rows(json(&format!("{{{first}}}"))))
        .await;
    reloader
        .reload_vault(&vault_rows(json(&format!("{{{second}}}"))))
        .await;
    assert_eq!(control.initialized().len(), 2);
}

#[tokio::test]
async fn a_missing_vault_row_clears_a_previously_applied_manager() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let (_logs, _guard) = capture_logs();
    reloader
        .reload_vault(&vault_rows(json("{\"vault_addr\":\"addr\"}")))
        .await;
    reloader.reload_vault(&FakeRows::default()).await;
    assert_eq!(env_value(&reloader, "HCP_VAULT_ADDR"), None);
    assert_eq!(control.dropped(), 1);
    assert_eq!(control.active(), None);
    assert_eq!(reloader.state.vault_last_config, None);
}

#[tokio::test]
async fn a_missing_vault_row_with_nothing_applied_does_nothing() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let (logs, _guard) = capture_logs();
    reloader.reload_vault(&FakeRows::default()).await;
    assert!(control.initialized().is_empty());
    assert_eq!(control.dropped(), 0);
    assert_eq!(logs.contents(), "");
}

#[tokio::test]
async fn a_vault_read_failure_is_logged() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let rows = FakeRows {
        fail: Some(fault("the database is down")),
        ..FakeRows::default()
    };
    let (logs, _guard) = capture_logs();
    reloader.reload_vault(&rows).await;
    assert_eq!(rows.calls(), 1);
    assert!(control.initialized().is_empty());
    assert!(logs.contents().contains(
        "Error loading Hashicorp Vault config override from DB: could not read the settings row: the database is down"
    ));
}

#[tokio::test]
async fn a_vault_transport_failure_retries_with_its_reason() {
    let control = ControlLog::default();
    let (reconnect, reasons) = FakeReconnect::new(ReconnectResult::Reconnected);
    let mut reloader = reloader_with(&control, Some(reconnect));
    let rows = FakeRows {
        vault: Some(json("{\"vault_addr\":\"addr\"}")),
        fail: Some(fault("the database is down")),
        fail_once: true,
        ..FakeRows::default()
    };
    let (_logs, _guard) = capture_logs();
    reloader.reload_vault(&rows).await;
    assert_eq!(rows.calls(), 2);
    assert_eq!(
        *reasons.lock().unwrap(),
        vec!["init_hashicorp_vault_config_override_lookup_failure".to_owned()]
    );
    assert_eq!(
        env_value(&reloader, "HCP_VAULT_ADDR"),
        Some("addr".to_owned())
    );
}

#[tokio::test]
async fn a_vault_failure_that_is_not_a_transport_error_is_not_retried() {
    let control = ControlLog::default();
    let (reconnect, reasons) = FakeReconnect::new(ReconnectResult::Reconnected);
    let mut reloader = reloader_with(&control, Some(reconnect));
    let rows = FakeRows {
        fail: Some(DbError {
            error_type: ErrorType::Other,
            ..fault("a data fault")
        }),
        ..FakeRows::default()
    };
    let (_logs, _guard) = capture_logs();
    reloader.reload_vault(&rows).await;
    assert_eq!(rows.calls(), 1);
    assert!(reasons.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_failing_vault_initialize_restores_the_previous_environment() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    reloader
        .environment
        .boot
        .insert("HCP_VAULT_ADDR".to_owned(), "boot-addr".to_owned());
    let cipher = cipher();
    let first = vault_config(&cipher, "addr1", "token1");
    let second = vault_config(&cipher, "addr2", "token2");
    let (logs, _guard) = capture_logs();
    reloader.reload_vault(&vault_rows(json(&first))).await;
    control.set_fail(true);
    reloader.reload_vault(&vault_rows(json(&second))).await;
    assert!(logs.contents().contains(
        "Error loading Hashicorp Vault config override from DB: the secret manager failed to initialize: the fake secret manager is down"
    ));
    assert_eq!(
        env_value(&reloader, "HCP_VAULT_ADDR"),
        Some("addr1".to_owned())
    );
    assert_eq!(
        env_value(&reloader, "HCP_VAULT_TOKEN"),
        Some("token1".to_owned())
    );
    assert_eq!(reloader.state.vault_last_config, Some(json(&first)));
    control.set_fail(false);
    reloader.reload_vault(&vault_rows(json(&second))).await;
    assert_eq!(
        env_value(&reloader, "HCP_VAULT_ADDR"),
        Some("addr2".to_owned())
    );
}

#[rstest]
#[case::array("[]")]
#[case::null("null")]
#[case::number("5")]
async fn a_stored_vault_value_that_is_not_an_object_is_logged(#[case] stored: &str) {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let (logs, _guard) = capture_logs();
    reloader.reload_vault(&vault_rows(json(stored))).await;
    assert!(control.initialized().is_empty());
    assert_eq!(reloader.state.vault_last_config, None);
    assert!(logs.contents().contains(VAULT_ERROR));
    assert!(
        logs.contents()
            .contains("the stored hashicorp_vault config override is valid JSON but not an object")
    );
}

#[tokio::test]
async fn a_vault_string_that_is_not_json_decodes_to_an_empty_config() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    reloader
        .environment
        .boot
        .insert("HCP_VAULT_ADDR".to_owned(), "boot-addr".to_owned());
    let row = JsonValue::String("not json".to_owned());
    let (_logs, _guard) = capture_logs();
    reloader.reload_vault(&vault_rows(row)).await;
    assert_eq!(env_value(&reloader, "HCP_VAULT_ADDR"), None);
    assert_eq!(
        control.initialized(),
        vec![KeyManagementSystem::HashicorpVault]
    );
    assert_eq!(reloader.state.vault_last_config, Some(json("{}")));
}

fn cyberark_environment(reloader: &mut SettingsReloader) {
    reloader
        .environment
        .boot
        .insert("CYBERARK_API_BASE".to_owned(), "boot-base".to_owned());
    reloader
        .environment
        .boot
        .insert("CYBERARK_ACCOUNT".to_owned(), "boot-account".to_owned());
}

#[tokio::test]
async fn a_cyberark_row_sets_the_mapped_environment_and_initializes_the_manager() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    cyberark_environment(&mut reloader);
    let config = format!(
        "{{\"cyberark_api_base\":\"new-base\",\"cyberark_api_key\":\"{}\"}}",
        sealed(&cipher(), "key")
    );
    let (logs, _guard) = capture_logs();
    reloader
        .reload_cyberark(&cyberark_rows(json(&config)))
        .await;
    assert_eq!(
        env_value(&reloader, "CYBERARK_API_BASE"),
        Some("new-base".to_owned())
    );
    assert_eq!(
        env_value(&reloader, "CYBERARK_API_KEY"),
        Some("key".to_owned())
    );
    assert_eq!(control.initialized(), vec![KeyManagementSystem::Cyberark]);
    assert_eq!(reloader.state.cyberark_last_config, Some(json(&config)));
    assert!(
        logs.contents()
            .contains("CyberArk config override loaded from DB")
    );
}

#[tokio::test]
async fn the_cyberark_boot_environment_is_snapshotted_once() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    cyberark_environment(&mut reloader);
    let (_logs, _guard) = capture_logs();
    reloader
        .reload_cyberark(&cyberark_rows(json("{\"cyberark_api_base\":\"one\"}")))
        .await;
    reloader
        .reload_cyberark(&cyberark_rows(json("{\"cyberark_api_base\":\"two\"}")))
        .await;
    let boot = reloader.state.cyberark_boot_env.as_ref().unwrap();
    assert_eq!(boot.get("cyberark_api_base"), Some(&json("\"boot-base\"")));
    assert_eq!(
        boot.get("cyberark_account"),
        Some(&json("\"boot-account\""))
    );
    assert_eq!(boot.get("cyberark_api_key"), Some(&JsonValue::Null));
}

#[tokio::test]
async fn a_missing_cyberark_row_restores_the_boot_environment() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    cyberark_environment(&mut reloader);
    let (_logs, _guard) = capture_logs();
    reloader
        .reload_cyberark(&cyberark_rows(json(
            "{\"cyberark_api_base\":\"one\",\"cyberark_username\":\"user\"}",
        )))
        .await;
    assert_eq!(
        env_value(&reloader, "CYBERARK_API_BASE"),
        Some("one".to_owned())
    );
    reloader.reload_cyberark(&FakeRows::default()).await;
    assert_eq!(
        env_value(&reloader, "CYBERARK_API_BASE"),
        Some("boot-base".to_owned())
    );
    assert_eq!(
        env_value(&reloader, "CYBERARK_ACCOUNT"),
        Some("boot-account".to_owned())
    );
    assert_eq!(env_value(&reloader, "CYBERARK_USERNAME"), None);
    assert_eq!(reloader.state.cyberark_last_config, None);
}

#[tokio::test]
async fn a_missing_cyberark_row_with_nothing_applied_does_nothing() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let (logs, _guard) = capture_logs();
    reloader.reload_cyberark(&FakeRows::default()).await;
    assert!(control.initialized().is_empty());
    assert_eq!(logs.contents(), "");
}

#[tokio::test]
async fn an_unchanged_cyberark_row_does_nothing() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let rows = cyberark_rows(json("{\"cyberark_api_base\":\"one\"}"));
    let (_logs, _guard) = capture_logs();
    reloader.reload_cyberark(&rows).await;
    control.reset();
    reloader.reload_cyberark(&rows).await;
    assert!(control.initialized().is_empty());
}

#[tokio::test]
async fn a_failing_cyberark_initialize_restores_the_previous_environment() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let cipher = cipher();
    let first = cyberark_config(&cipher, "one", "key1");
    let second = cyberark_config(&cipher, "two", "key2");
    let (logs, _guard) = capture_logs();
    reloader.reload_cyberark(&cyberark_rows(json(&first))).await;
    control.set_fail(true);
    reloader
        .reload_cyberark(&cyberark_rows(json(&second)))
        .await;
    assert!(logs.contents().contains(CYBERARK_ERROR));
    assert_eq!(
        env_value(&reloader, "CYBERARK_API_BASE"),
        Some("one".to_owned())
    );
    assert_eq!(
        env_value(&reloader, "CYBERARK_API_KEY"),
        Some("key1".to_owned())
    );
    assert_eq!(reloader.state.cyberark_last_config, Some(json(&first)));
}

#[tokio::test]
async fn a_cyberark_read_failure_is_logged() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let rows = FakeRows {
        fail: Some(fault("the database is down")),
        ..FakeRows::default()
    };
    let (logs, _guard) = capture_logs();
    reloader.reload_cyberark(&rows).await;
    assert!(logs.contents().contains(
        "Error loading CyberArk config override from DB: could not read the settings row: the database is down"
    ));
    assert!(control.initialized().is_empty());
}

#[tokio::test]
async fn a_cyberark_transport_failure_retries_with_its_reason() {
    let control = ControlLog::default();
    let (reconnect, reasons) = FakeReconnect::new(ReconnectResult::Reconnected);
    let mut reloader = reloader_with(&control, Some(reconnect));
    let rows = FakeRows {
        cyberark: Some(json("{\"cyberark_api_base\":\"one\"}")),
        fail: Some(fault("the database is down")),
        fail_once: true,
        ..FakeRows::default()
    };
    let (_logs, _guard) = capture_logs();
    reloader.reload_cyberark(&rows).await;
    assert_eq!(rows.calls(), 2);
    assert_eq!(
        *reasons.lock().unwrap(),
        vec!["init_cyberark_config_override_lookup_failure".to_owned()]
    );
    assert_eq!(
        env_value(&reloader, "CYBERARK_API_BASE"),
        Some("one".to_owned())
    );
}

#[tokio::test]
async fn a_cyberark_row_leaves_the_vault_state_alone() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let (_logs, _guard) = capture_logs();
    reloader
        .reload_cyberark(&cyberark_rows(json("{\"cyberark_api_base\":\"one\"}")))
        .await;
    assert_eq!(reloader.state.vault_last_config, None);
    assert_eq!(env_value(&reloader, "HCP_VAULT_ADDR"), None);
}
