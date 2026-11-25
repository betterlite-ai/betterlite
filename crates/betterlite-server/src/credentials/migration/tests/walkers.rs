#![cfg(test)]

use betterlite_db::encryption_scan::MigrationJson;
use betterlite_db::encryption_scan::ScanError;
use betterlite_secrets::secret_codec::Algorithm;

use super::super::error::MigrationError;
use super::super::service::{Migration, check_encryption};
use super::super::store::CallbackTable;
use super::support::{
    FailingTransform, FakeError, FakeRotation, FakeStore, RotationCall, SALT_KEY, Write,
    callback_row, callback_settings_metadata, callback_text, callback_vars_metadata, cipher,
    covered_report, field_text, legacy, location_of, marked, migration, object, open, secret, v2,
};

#[tokio::test]
async fn migrate_rejects_a_disabled_gate_before_reading_any_row() {
    let store = FakeStore::default();
    let mut rotation = FakeRotation::default();
    let cipher = cipher(Algorithm::XSalsa20Poly1305, Some(SALT_KEY));
    let error = migration(&store, &mut rotation, &cipher, false)
        .run()
        .await
        .expect_err("the gate refuses the migration");
    assert!(matches!(error, MigrationError::AlgorithmGate));
    assert!(store.queued_scans().is_empty());
    assert!(store.writes().is_empty());
    assert!(rotation.calls().is_empty());
}

#[tokio::test]
async fn dry_run_scans_every_location_in_source_order_without_rotating_or_writing() {
    let store = FakeStore::default();
    store
        .scans
        .lock()
        .unwrap()
        .push_back(covered_report([2, 1, 0, 0, 0, 0]));
    store.configs.lock().unwrap().insert(
        "vantage_settings".to_owned(),
        object(vec![
            ("api_key", secret(&legacy(SALT_KEY, "sk-vantage"))),
            ("integration_token", secret(&v2(SALT_KEY, "token-vantage"))),
            ("base_url", secret("https://vantage.test")),
        ]),
    );
    let cloudzero = serde_json::to_string(&object(vec![(
        "api_key",
        secret(&legacy(SALT_KEY, "sk-cloudzero")),
    )]))
    .expect("the cloudzero fixture is JSON");
    store
        .configs
        .lock()
        .unwrap()
        .insert("cloudzero_settings".to_owned(), secret(&cloudzero));
    store.teams.lock().unwrap().push(callback_row(
        "team-1",
        callback_vars_metadata(vec![
            ("api_key", marked(&legacy(SALT_KEY, "sk-team"))),
            ("other", marked(&v2(SALT_KEY, "token-team"))),
            ("label", "plain".to_owned()),
        ]),
    ));
    let mut rotation = FakeRotation::default();
    let cipher = cipher(Algorithm::Aes256Gcm, Some(SALT_KEY));
    let report = migration(&store, &mut rotation, &cipher, true)
        .run()
        .await
        .expect("the dry run reports");
    let names = report
        .locations
        .iter()
        .map(|location| location.location.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            "model_table",
            "credentials",
            "mcp_server",
            "mcp_user_credentials",
            "mcp_user_env_vars",
            "config_environment_variables",
            "team.callback_vars",
            "verification_token.callback_vars",
            "vantage_settings",
            "cloudzero_settings",
            "sso_config",
        ]
    );
    let team = location_of(&report, "team.callback_vars");
    assert_eq!(
        (team.scanned, team.legacy, team.already_v2, team.plaintext),
        (3, 1, 1, 1)
    );
    let vantage = location_of(&report, "vantage_settings");
    assert_eq!(
        (vantage.scanned, vantage.legacy, vantage.already_v2),
        (2, 1, 1)
    );
    assert_eq!(location_of(&report, "cloudzero_settings").legacy, 1);
    assert_eq!(location_of(&report, "model_table").legacy, 2);
    assert_eq!(
        report
            .as_dict()
            .expect("the report projects")
            .residual_legacy,
        6
    );
    assert!(rotation.calls().is_empty());
    assert!(store.writes().is_empty());
    assert!(store.queued_scans().is_empty());
}

#[tokio::test]
async fn check_encryption_attests_marked_callback_values_with_the_gate_off() {
    let store = FakeStore::default();
    store
        .scans
        .lock()
        .unwrap()
        .push_back(covered_report([0, 0, 0, 0, 0, 0]));
    store.teams.lock().unwrap().extend([
        callback_row(
            "team-marked",
            callback_vars_metadata(vec![("api_key", marked(&legacy(SALT_KEY, "sk-live")))]),
        ),
        callback_row(
            "team-v2",
            callback_settings_metadata(vec![("api_key", marked(&v2(SALT_KEY, "sk-done")))]),
        ),
        callback_row("team-string", secret("{\"logging\":[]}")),
        callback_row("team-no-slots", object(vec![("other", secret("value"))])),
    ]);
    let cipher = cipher(Algorithm::XSalsa20Poly1305, Some(SALT_KEY));
    let report = check_encryption(&store, &cipher)
        .await
        .expect("the attestation runs without the gate");
    let team = location_of(&report, "team.callback_vars");
    assert_eq!((team.scanned, team.legacy, team.already_v2), (2, 1, 1));
    assert_eq!(
        location_of(&report, "verification_token.callback_vars").scanned,
        0
    );
    assert!(store.writes().is_empty());
}

#[tokio::test]
async fn real_run_brackets_the_same_key_rotation_and_derives_migrated_counts() {
    let store = FakeStore::default();
    store.scans.lock().unwrap().extend([
        covered_report([3, 1, 0, 0, 0, 0]),
        covered_report([1, 3, 0, 0, 0, 0]),
    ]);
    let mut rotation = FakeRotation::default();
    let cipher = cipher(Algorithm::Aes256Gcm, Some(SALT_KEY));
    let report = migration(&store, &mut rotation, &cipher, false)
        .run()
        .await
        .expect("the migration reports");
    let model = location_of(&report, "model_table");
    assert_eq!((model.legacy, model.migrated), (1, 2));
    let credentials = location_of(&report, "credentials");
    assert_eq!((credentials.legacy, credentials.migrated), (3, 0));
    assert_eq!(
        rotation.calls(),
        [RotationCall {
            key: SALT_KEY.to_owned(),
            actor: Some("admin".to_owned()),
        }]
    );
    assert!(store.queued_scans().is_empty());
}

#[tokio::test]
async fn a_real_run_without_a_salt_key_stops_after_the_first_scan() {
    let store = FakeStore::default();
    store.scans.lock().unwrap().extend([
        covered_report([1, 0, 0, 0, 0, 0]),
        covered_report([0, 0, 0, 0, 0, 0]),
    ]);
    let mut rotation = FakeRotation::default();
    let cipher = cipher(Algorithm::Aes256Gcm, None);
    let error = migration(&store, &mut rotation, &cipher, false)
        .run()
        .await
        .expect_err("the migration needs a key");
    assert!(matches!(error, MigrationError::MissingKey));
    assert_eq!(store.queued_scans().len(), 1);
    assert!(rotation.calls().is_empty());
}

#[tokio::test]
async fn a_rotation_failure_propagates_before_the_second_scan() {
    let store = FakeStore::default();
    store.scans.lock().unwrap().extend([
        covered_report([1, 0, 0, 0, 0, 0]),
        covered_report([0, 0, 0, 0, 0, 0]),
    ]);
    let mut rotation = FakeRotation::default();
    rotation.failure = true;
    let cipher = cipher(Algorithm::Aes256Gcm, Some(SALT_KEY));
    let error = migration(&store, &mut rotation, &cipher, false)
        .run()
        .await
        .expect_err("the rotation failure stops the migration");
    assert!(matches!(error, MigrationError::Rotation(_)));
    assert_eq!(store.queued_scans().len(), 1);
}

#[tokio::test]
async fn a_failed_callback_transform_counts_the_legacy_values_undecryptable() {
    let store = FakeStore::default();
    store.scans.lock().unwrap().extend([
        covered_report([0, 0, 0, 0, 0, 0]),
        covered_report([0, 0, 0, 0, 0, 0]),
    ]);
    store.teams.lock().unwrap().push(callback_row(
        "team-1",
        callback_vars_metadata(vec![
            ("api_key", marked(&legacy(SALT_KEY, "sk-a"))),
            ("secret_key", marked(&legacy(SALT_KEY, "sk-b"))),
        ]),
    ));
    let mut rotation = FakeRotation::default();
    let cipher = cipher(Algorithm::Aes256Gcm, Some(SALT_KEY));
    let mut service = Migration {
        store: &store,
        rotation: &mut rotation,
        cipher: &cipher,
        transform: FailingTransform,
        actor: None,
        dry_run: false,
    };
    let report = service
        .run()
        .await
        .expect("a transform failure does not stop the migration");
    let team = location_of(&report, "team.callback_vars");
    assert_eq!(
        (team.scanned, team.migrated, team.undecryptable, team.legacy),
        (2, 0, 2, 0)
    );
    assert!(store.writes().is_empty());
}

#[tokio::test]
async fn a_real_run_writes_the_reencrypted_callback_mapping_only_when_legacy_exists() {
    let store = FakeStore::default();
    store.scans.lock().unwrap().extend([
        covered_report([0, 0, 0, 0, 0, 0]),
        covered_report([0, 0, 0, 0, 0, 0]),
    ]);
    store.teams.lock().unwrap().extend([
        callback_row(
            "team-1",
            callback_vars_metadata(vec![
                ("api_key", marked(&legacy(SALT_KEY, "sk-live"))),
                ("endpoint", "https://example.test".to_owned()),
            ]),
        ),
        callback_row(
            "team-clean",
            callback_settings_metadata(vec![("api_key", marked(&v2(SALT_KEY, "sk-done")))]),
        ),
    ]);
    let mut rotation = FakeRotation::default();
    let cipher = cipher(Algorithm::Aes256Gcm, Some(SALT_KEY));
    let report = migration(&store, &mut rotation, &cipher, false)
        .run()
        .await
        .expect("the migration writes the row");
    let team = location_of(&report, "team.callback_vars");
    assert_eq!((team.scanned, team.migrated, team.already_v2), (3, 1, 1));
    let writes = store.writes();
    assert_eq!(writes.len(), 1);
    let Write::Callback {
        table,
        key,
        metadata,
    } = &writes[0]
    else {
        panic!("the team row is written");
    };
    assert_eq!(*table, CallbackTable::Team);
    assert_eq!(key, "team-1");
    let sealed_value = callback_text(metadata, "api_key");
    let inner = sealed_value
        .strip_prefix("litellm_enc::")
        .expect("the migrated value keeps the marker");
    assert!(inner.starts_with("v2:gcm:"));
    assert_eq!(open(SALT_KEY, inner).as_deref(), Some("sk-live"));
    assert_eq!(callback_text(metadata, "endpoint"), "https://example.test");
}

#[tokio::test]
async fn a_config_row_stored_as_json_text_is_decoded_and_rewritten() {
    let store = FakeStore::default();
    store.scans.lock().unwrap().extend([
        covered_report([0, 0, 0, 0, 0, 0]),
        covered_report([0, 0, 0, 0, 0, 0]),
    ]);
    let inner = serde_json::to_string(&object(vec![
        ("api_key", secret(&legacy(SALT_KEY, "sk-live"))),
        ("integration_token", secret(&v2(SALT_KEY, "kept-token"))),
    ]))
    .expect("the vantage fixture is JSON");
    store
        .configs
        .lock()
        .unwrap()
        .insert("vantage_settings".to_owned(), secret(&inner));
    let mut rotation = FakeRotation::default();
    let cipher = cipher(Algorithm::Aes256Gcm, Some(SALT_KEY));
    let report = migration(&store, &mut rotation, &cipher, false)
        .run()
        .await
        .expect("the migration writes the row");
    let vantage = location_of(&report, "vantage_settings");
    assert_eq!(
        (vantage.scanned, vantage.migrated, vantage.already_v2),
        (2, 1, 1)
    );
    let writes = store.writes();
    assert_eq!(writes.len(), 1);
    let Write::Config { param_name, value } = &writes[0] else {
        panic!("the config row is written");
    };
    assert_eq!(param_name, "vantage_settings");
    let api_key = field_text(value, "api_key").expect("api_key stays text");
    assert!(api_key.starts_with("v2:gcm:"));
    assert_eq!(open(SALT_KEY, &api_key).as_deref(), Some("sk-live"));
    assert_eq!(
        field_text(value, "integration_token").as_deref(),
        Some(v2(SALT_KEY, "kept-token").as_str())
    );
}

#[tokio::test]
async fn invalid_json_inside_a_config_row_propagates() {
    let store = FakeStore::default();
    store.scans.lock().unwrap().extend([
        covered_report([0, 0, 0, 0, 0, 0]),
        covered_report([0, 0, 0, 0, 0, 0]),
    ]);
    store
        .configs
        .lock()
        .unwrap()
        .insert("vantage_settings".to_owned(), secret("{not-json"));
    let mut rotation = FakeRotation::default();
    let cipher = cipher(Algorithm::Aes256Gcm, Some(SALT_KEY));
    let error = migration(&store, &mut rotation, &cipher, false)
        .run()
        .await
        .expect_err("invalid stored JSON stops the migration");
    assert!(matches!(
        error,
        MigrationError::Store(FakeError::Scan(ScanError::Json(_)))
    ));
    assert!(store.writes().is_empty());
}

#[tokio::test]
async fn sso_visits_every_nonempty_string_field_and_skips_the_rest() {
    let store = FakeStore::default();
    store.scans.lock().unwrap().extend([
        covered_report([0, 0, 0, 0, 0, 0]),
        covered_report([0, 0, 0, 0, 0, 0]),
    ]);
    *store.sso.lock().unwrap() = Some(object(vec![
        ("client_secret", secret(&legacy(SALT_KEY, "sso-secret"))),
        ("client_id", secret("plain-client")),
        ("already", secret(&v2(SALT_KEY, "sso-done"))),
        ("empty", secret("")),
        ("count", MigrationJson::Bool(true)),
    ]));
    let mut rotation = FakeRotation::default();
    let cipher = cipher(Algorithm::Aes256Gcm, Some(SALT_KEY));
    let report = migration(&store, &mut rotation, &cipher, false)
        .run()
        .await
        .expect("the migration writes the sso row");
    let sso = location_of(&report, "sso_config");
    assert_eq!(
        (sso.scanned, sso.migrated, sso.already_v2, sso.plaintext),
        (3, 1, 1, 1)
    );
    let writes = store.writes();
    assert_eq!(writes.len(), 1);
    let Write::Sso(value) = &writes[0] else {
        panic!("the sso row is written");
    };
    let secret_value = field_text(value, "client_secret").expect("client_secret stays text");
    assert!(secret_value.starts_with("v2:gcm:"));
    assert_eq!(open(SALT_KEY, &secret_value).as_deref(), Some("sso-secret"));
    assert_eq!(
        field_text(value, "client_id").as_deref(),
        Some("plain-client")
    );
    assert_eq!(
        field_text(value, "already").as_deref(),
        Some(v2(SALT_KEY, "sso-done").as_str())
    );
    assert_eq!(field_text(value, "empty").as_deref(), Some(""));
    assert!(
        matches!(value, MigrationJson::Object(fields) if matches!(fields.get("count"), Some(MigrationJson::Bool(true))))
    );
}
