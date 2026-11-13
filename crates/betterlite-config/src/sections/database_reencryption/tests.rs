#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use betterlite_db::database_reencryption::MigrationError;
use betterlite_db::database_reencryption::outcome::{MigrationOutcome, NothingToMigrate};
use betterlite_db::database_reencryption::registry::SecretColumn;
use betterlite_db::database_reencryption::runner::{MigrationRequest, migrate_if_requested};
use betterlite_db::database_reencryption::storage::{SecretRow, SecretStorage};
use betterlite_db::database_reencryption::traversal::replace_ciphertexts;
use betterlite_db::encryption_scan::MigrationJson;
use betterlite_db::encryption_scan::decode_migration_json;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use crate::loader::{ConfigError, ConfigLoader};

fn loaded(environment: &[(&str, &str)]) -> Result<super::Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let config = ConfigLoader { lookup }.load("{}")?;
    Ok(config.sections.database_reencryption)
}

fn decoded(text: &str) -> MigrationJson {
    decode_migration_json(text.as_bytes()).expect("the fixture is JSON")
}

fn rendered(value: &MigrationJson) -> String {
    serde_json::to_string(value).expect("the value serializes")
}

fn wrapped(depth: usize) -> MigrationJson {
    decoded(&format!(
        "{}{}{}",
        "[".repeat(depth),
        "\"secret\"",
        "]".repeat(depth)
    ))
}

fn replaced_at(depth: i64, value: MigrationJson) -> String {
    let mut replacement = |text: &str| Ok::<_, MigrationError>(Some(format!("<{text}>")));
    match replace_ciphertexts(value, &mut replacement, 0, depth) {
        Ok((replaced, _)) => rendered(&replaced),
        Err(error) => panic!("the traversal failed: {error}"),
    }
}

fn cipher() -> DbCipher {
    DbCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from("configured-salt")),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

#[derive(Default)]
struct NoRows {
    read: AtomicBool,
}

impl SecretStorage for NoRows {
    fn secret_columns(
        &self,
    ) -> impl Future<Output = Result<Vec<SecretColumn>, MigrationError>> + Send {
        self.read.store(true, Ordering::SeqCst);
        ready(Ok(Vec::new()))
    }

    fn rows(
        &self,
        _column: &SecretColumn,
    ) -> impl Future<Output = Result<Vec<SecretRow>, MigrationError>> + Send {
        self.read.store(true, Ordering::SeqCst);
        ready(Ok(Vec::new()))
    }

    fn replace(
        &self,
        _column: &SecretColumn,
        _old: &str,
        _new: &str,
        _primary_key: &str,
    ) -> impl Future<Output = Result<bool, MigrationError>> + Send {
        self.read.store(true, Ordering::SeqCst);
        ready(Ok(false))
    }
}

struct Run {
    outcome: Option<MigrationOutcome>,
    storage: Arc<NoRows>,
    connected: bool,
}

async fn outcome_for(
    settings: &super::Settings,
    previous_master_key: Option<&SecretString>,
    master_key: Option<&SecretString>,
) -> Run {
    let storage = Arc::new(NoRows::default());
    let connected = Arc::new(AtomicBool::new(false));
    let cipher = cipher();
    let request = MigrationRequest {
        previous_master_key,
        master_key,
        salt_key_is_set: settings.salt_key_is_set,
        cipher: &cipher,
        max_depth: settings.max_recurse_depth,
    };
    let connected_flag = Arc::clone(&connected);
    let database = storage.as_ref();
    let outcome = migrate_if_requested(
        &request,
        move || {
            connected_flag.store(true, Ordering::SeqCst);
            Ok(Some(database))
        },
        |_| Ok::<(), MigrationError>(()),
        Ok,
    )
    .await;
    Run {
        outcome: outcome.ok().flatten(),
        storage,
        connected: connected.load(Ordering::SeqCst),
    }
}

#[test]
fn recurse_depth_default_hundred() {
    let settings = loaded(&[]).expect("the default loads");
    assert_eq!(settings.max_recurse_depth, 100);
    assert_eq!(
        replaced_at(settings.max_recurse_depth, wrapped(101)),
        rendered(&wrapped(101)),
        "a leaf one level past the loaded depth stays"
    );
    assert_ne!(
        replaced_at(settings.max_recurse_depth, wrapped(100)),
        rendered(&wrapped(100)),
        "a leaf exactly at the loaded depth is replaced"
    );
    let lookup: BTreeMap<String, String> = BTreeMap::new();
    let alias = ConfigLoader { lookup }.load("DEFAULT_MAX_RECURSE_DEPTH: 5\n");
    assert!(
        matches!(alias, Err(ConfigError::Unsupported { .. })),
        "the setting has no YAML alias"
    );
}

#[rstest]
#[case::signed("+7", 7, "\"<leaf>\"", "[\"<leaf>\"]")]
#[case::whitespace(" 9 ", 9, "\"<leaf>\"", "[\"<leaf>\"]")]
#[case::negative("-4", -4, "\"leaf\"", "[\"leaf\"]")]
#[case::zero("0", 0, "\"<leaf>\"", "[\"leaf\"]")]
fn recurse_depth_env_signed_and_whitespace(
    #[case] raw: &str,
    #[case] expected: i64,
    #[case] root: &str,
    #[case] child: &str,
) {
    let settings = loaded(&[("DEFAULT_MAX_RECURSE_DEPTH", raw)]).expect("a signed integer loads");
    assert_eq!(settings.max_recurse_depth, expected);
    assert_eq!(replaced_at(expected, decoded("\"leaf\"")), root);
    assert_eq!(replaced_at(expected, decoded("[\"leaf\"]")), child);
}

#[rstest]
#[case::text("many")]
#[case::fraction("1.5")]
#[case::overflow("9223372036854775808")]
fn recurse_depth_invalid_stops_startup(#[case] raw: &str) {
    let error = loaded(&[("DEFAULT_MAX_RECURSE_DEPTH", raw)]).expect_err("startup stops");
    assert!(
        matches!(error, ConfigError::Environment { .. }),
        "{error:?}"
    );
}

#[test]
fn recurse_depth_empty_stops_startup() {
    let error = loaded(&[("DEFAULT_MAX_RECURSE_DEPTH", "")]).expect_err("startup stops");
    assert!(
        matches!(error, ConfigError::Environment { .. }),
        "{error:?}"
    );
}

#[tokio::test]
async fn migration_key_absent_vs_empty() {
    let absent = loaded(&[]).expect("the default loads");
    let master = SecretString::from("sk-new");
    let result = outcome_for(&absent, None, Some(&master)).await;
    assert!(result.outcome.is_none());
    assert!(
        !result.connected,
        "no database callback runs without a request"
    );
    assert!(!result.storage.read.load(Ordering::SeqCst));

    let empty = loaded(&[("LITELLM_MIGRATE_FROM_MASTER_KEY", "")]).expect("empty is present");
    assert_eq!(
        empty
            .migrate_from_master_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        Some(String::new())
    );
    let empty_key = SecretString::from("");
    let empty_result = outcome_for(&empty, Some(&empty_key), Some(&master)).await;
    assert!(
        empty_result.connected,
        "an empty variable is still a request"
    );
    assert!(matches!(
        empty_result.outcome,
        Some(MigrationOutcome::Nothing(
            NothingToMigrate::NothingLeftToMigrate
        ))
    ));
}

#[tokio::test]
async fn salt_present_empty_skips() {
    let settings = loaded(&[("LITELLM_SALT_KEY", "")]).expect("an empty salt is present");
    assert!(settings.salt_key_is_set);
    let previous = SecretString::from("sk-old");
    let master = SecretString::from("sk-new");
    let result = outcome_for(&settings, Some(&previous), Some(&master)).await;
    assert!(
        result.connected,
        "the writer is obtained before the salt decides"
    );
    assert!(
        !result.storage.read.load(Ordering::SeqCst),
        "salt skips all reads"
    );
    assert!(matches!(
        result.outcome,
        Some(MigrationOutcome::Nothing(
            NothingToMigrate::SaltKeyEncryptsStoredValues
        ))
    ));
}
