#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::sync::Arc;
use std::time::Duration;

use rstest::rstest;
use secrecy::ExposeSecret;
use tempfile::NamedTempFile;
use tracing::subscriber::with_default;

use super::IGNORED_BOOTSTRAP_INPUTS;
use super::startup::StartupSettings;
use crate::loader::{ConfigError, ConfigLoader};
use crate::types::Config;

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn config(source: &str, pairs: &[(&str, &str)]) -> Config {
    ConfigLoader {
        lookup: environment(pairs),
    }
    .load(source)
    .expect("the config should load")
}

fn logged<T, F>(action: F) -> (T, String)
where
    F: FnOnce() -> T,
{
    let log = NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    let result = with_default(subscriber, action);
    (result, fs::read_to_string(log.path()).unwrap())
}

fn fallback_warnings(log: &str) -> usize {
    log.matches("falling back to").count()
}

#[test]
fn loads_settings() {
    let source = "general_settings:\n  disable_prisma_schema_update: false\n";
    let pairs = [
        ("DISABLE_SCHEMA_UPDATE", "true"),
        ("DATABASE_URL", "postgresql://writer/db?schema=tenant"),
        ("DIRECT_URL", "postgresql://direct/db"),
        ("USE_V2_MIGRATION_RESOLVER", "false"),
        ("USE_PRISMA_DB_PUSH", "true"),
        ("LITELLM_SET_REPLICA_IDENTITY_FULL", "true"),
        ("LITELLM_PRISMA_COMMAND_TIMEOUT", "90"),
        ("LITELLM_MIGRATION_LOCK_TIMEOUT", "450"),
        ("LITELLM_PRISMA_MIGRATE_DEPLOY_TIMEOUT", "1200"),
        ("PRISMA_OFFLINE_MODE", "true"),
        ("PRISMA_CLI_PATH", "/opt/prisma"),
        ("PRISMA_NODEENV_CACHE_DIR", "/cache"),
        ("LITELLM_PRISMA_BOOTSTRAP_TIMEOUT", "900"),
        ("LITELLM_MIGRATION_DIR", "/migrations"),
    ];
    let loaded = config(source, &pairs);
    let settings = &loaded.sections.database_startup;
    assert!(
        settings.updates_enabled,
        "an explicit false enables updates"
    );
    assert_eq!(settings.command_timeout, Duration::from_secs(90));
    assert_eq!(settings.lock_timeout, Duration::from_secs(450));
    assert_eq!(settings.deploy_timeout, Duration::from_mins(20));
    assert_eq!(settings.bootstrap_inputs, IGNORED_BOOTSTRAP_INPUTS.to_vec());
    assert_startup_view(&loaded);
}

fn assert_startup_view(loaded: &Config) {
    let startup = StartupSettings::from_config(loaded);
    assert!(startup.updates_enabled.0);
    assert_eq!(
        startup
            .database_url
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("postgresql://writer/db?schema=tenant")
    );
    assert_eq!(
        startup.direct_url.as_ref().map(ExposeSecret::expose_secret),
        Some("postgresql://direct/db")
    );
    assert!(!startup.use_v2_migration_resolver.0);
    assert!(startup.use_prisma_db_push.0);
    assert!(startup.set_replica_identity_full.0);
    assert_eq!(startup.command_timeout, Duration::from_secs(90));
    assert_eq!(startup.lock_timeout, Duration::from_secs(450));
    assert_eq!(startup.deploy_timeout, Duration::from_mins(20));
    assert_eq!(startup.bootstrap_inputs, IGNORED_BOOTSTRAP_INPUTS.to_vec());
}

#[rstest]
#[case::setting_false_beats_enabled_environment(
    "general_settings:\n  disable_prisma_schema_update: false\n",
    &[("DISABLE_SCHEMA_UPDATE", "true")],
    true
)]
#[case::setting_true_beats_disabled_environment(
    "general_settings:\n  disable_prisma_schema_update: true\n",
    &[("DISABLE_SCHEMA_UPDATE", "false")],
    false
)]
#[case::explicit_null_reads_the_environment(
    "general_settings:\n  disable_prisma_schema_update: null\n",
    &[("DISABLE_SCHEMA_UPDATE", "yes")],
    false
)]
#[case::absent_reads_the_environment(
    "general_settings: {}\n",
    &[("DISABLE_SCHEMA_UPDATE", "true")],
    false
)]
#[case::nothing_set_enables_updates("general_settings: {}\n", &[], true)]
#[case::empty_environment_counts_as_unset(
    "general_settings: {}\n",
    &[("DISABLE_SCHEMA_UPDATE", "")],
    true
)]
#[case::quoted_true_disables_updates(
    "general_settings:\n  disable_prisma_schema_update: \"TRUE\"\n",
    &[],
    false
)]
#[case::padded_true_disables_updates(
    "general_settings:\n  disable_prisma_schema_update: \" true \"\n",
    &[],
    false
)]
#[case::quoted_yes_keeps_updates(
    "general_settings:\n  disable_prisma_schema_update: \"yes\"\n",
    &[],
    true
)]
#[case::quoted_one_keeps_updates(
    "general_settings:\n  disable_prisma_schema_update: \"1\"\n",
    &[],
    true
)]
#[case::literal_on_is_not_true(
    "general_settings:\n  disable_prisma_schema_update: \"on\"\n",
    &[],
    true
)]
#[case::literal_unreadable_text_enables_updates(
    "general_settings:\n  disable_prisma_schema_update: \"maybe\"\n",
    &[],
    true
)]
#[case::environment_on_is_true(
    "general_settings: {}\n",
    &[("DISABLE_SCHEMA_UPDATE", "on")],
    false
)]
#[case::referenced_setting_reads_its_variable(
    "general_settings:\n  disable_prisma_schema_update: \"os.environ/SCHEMA_UPDATES_OFF\"\n",
    &[("SCHEMA_UPDATES_OFF", "1")],
    false
)]
#[case::reference_without_a_value_reads_the_setting_variable(
    "general_settings:\n  disable_prisma_schema_update: \"os.environ/SCHEMA_UPDATES_OFF\"\n",
    &[("DISABLE_SCHEMA_UPDATE", "true")],
    false
)]
#[case::unset_reference_enables_updates(
    "general_settings:\n  disable_prisma_schema_update: \"os.environ/SCHEMA_UPDATES_OFF\"\n",
    &[],
    true
)]
fn defaults_and_precedence(
    #[case] source: &str,
    #[case] pairs: &[(&str, &str)],
    #[case] updates_enabled: bool,
) {
    let defaults = config("general_settings: {}\n", &[])
        .sections
        .database_startup;
    assert!(defaults.updates_enabled);
    assert_eq!(defaults.command_timeout, Duration::from_secs(60));
    assert_eq!(defaults.lock_timeout, Duration::from_secs(600));
    assert_eq!(defaults.deploy_timeout, Duration::from_secs(600));
    assert!(defaults.bootstrap_inputs.is_empty());

    let loaded = config(source, pairs).sections.database_startup;
    assert_eq!(
        loaded.updates_enabled, updates_enabled,
        "{source} with {pairs:?}"
    );
}

#[test]
fn timeout_defaults_and_precedence() {
    let inherited = config(
        "general_settings: {}\n",
        &[("LITELLM_PRISMA_COMMAND_TIMEOUT", "900")],
    )
    .sections
    .database_startup;
    assert_eq!(inherited.command_timeout, Duration::from_mins(15));
    assert_eq!(inherited.deploy_timeout, Duration::from_mins(15));

    let pinned_deploy = config(
        "general_settings: {}\n",
        &[
            ("LITELLM_PRISMA_COMMAND_TIMEOUT", "900"),
            ("LITELLM_PRISMA_MIGRATE_DEPLOY_TIMEOUT", "1200"),
        ],
    )
    .sections
    .database_startup;
    assert_eq!(pinned_deploy.deploy_timeout, Duration::from_mins(20));

    let empty_deploy = config(
        "general_settings: {}\n",
        &[
            ("LITELLM_PRISMA_COMMAND_TIMEOUT", "900"),
            ("LITELLM_PRISMA_MIGRATE_DEPLOY_TIMEOUT", ""),
        ],
    )
    .sections
    .database_startup;
    // an empty deploy variable is unset, so the command budget carries over
    assert_eq!(empty_deploy.deploy_timeout, Duration::from_mins(15));
}

#[test]
fn invalid_deploy_timeout_warns() {
    let (invalid_deploy, log) = logged(|| {
        config(
            "general_settings: {}\n",
            &[
                ("LITELLM_PRISMA_COMMAND_TIMEOUT", "900"),
                ("LITELLM_PRISMA_MIGRATE_DEPLOY_TIMEOUT", "soon"),
            ],
        )
        .sections
        .database_startup
    });
    // an unusable deploy value never inherits the command budget, unlike an absent one
    assert_eq!(invalid_deploy.deploy_timeout, Duration::from_secs(600));
    // the migration_ledger section reads the deploy variable too, so the warning appears twice
    assert_eq!(fallback_warnings(&log), 2);
}

#[rstest]
#[case::text("abc")]
#[case::zero("0")]
#[case::negative("-5")]
#[case::infinite("inf")]
#[case::not_a_number("NaN")]
fn invalid_value_fallback(#[case] invalid: &str) {
    let (settings, log) = logged(|| {
        config(
            "general_settings: {}\n",
            &[
                ("LITELLM_PRISMA_COMMAND_TIMEOUT", invalid),
                ("LITELLM_MIGRATION_LOCK_TIMEOUT", invalid),
                ("LITELLM_PRISMA_MIGRATE_DEPLOY_TIMEOUT", invalid),
            ],
        )
        .sections
        .database_startup
    });
    assert_eq!(
        settings.command_timeout,
        Duration::from_secs(60),
        "command timeout for {invalid}"
    );
    assert_eq!(
        settings.lock_timeout,
        Duration::from_secs(600),
        "lock timeout for {invalid}"
    );
    assert_eq!(
        settings.deploy_timeout,
        Duration::from_secs(600),
        "deploy timeout for {invalid}"
    );
    // the migration_ledger section reads the same three variables, so each unusable value warns twice
    assert_eq!(fallback_warnings(&log), 6, "warnings for {invalid}");
    assert!(log.contains("LITELLM_PRISMA_COMMAND_TIMEOUT"), "{log}");
    assert!(log.contains("LITELLM_MIGRATION_LOCK_TIMEOUT"), "{log}");
    assert!(
        log.contains("LITELLM_PRISMA_MIGRATE_DEPLOY_TIMEOUT"),
        "{log}"
    );
}

#[test]
fn invalid_command_timeout_warns() {
    let (inherited, log) = logged(|| {
        config(
            "general_settings: {}\n",
            &[("LITELLM_PRISMA_COMMAND_TIMEOUT", "abc")],
        )
        .sections
        .database_startup
    });
    assert_eq!(inherited.deploy_timeout, Duration::from_secs(600));
    // the deploy read inherits the command budget, so an unusable command warns twice per section
    assert_eq!(fallback_warnings(&log), 4);
}

#[test]
fn invalid_values_name_their_source() {
    let list = load_error(
        "general_settings:\n  disable_prisma_schema_update: [true]\n",
        &[],
    );
    assert!(
        matches!(
            &list,
            ConfigError::InvalidScalar { path, .. }
                if path.as_str() == "general_settings.disable_prisma_schema_update"
        ),
        "{list}"
    );

    let environment_error = load_error(
        "general_settings: {}\n",
        &[("DISABLE_SCHEMA_UPDATE", "maybe")],
    );
    assert!(
        matches!(
            &environment_error,
            ConfigError::Environment { source }
                if source.to_string().contains("DISABLE_SCHEMA_UPDATE")
        ),
        "{environment_error}"
    );

    let reference = load_error(
        "general_settings:\n  disable_prisma_schema_update: os.environ/SCHEMA_UPDATES_OFF\n",
        &[("SCHEMA_UPDATES_OFF", "maybe")],
    );
    assert!(
        matches!(
            &reference,
            ConfigError::Environment { source }
                if source.to_string().contains("SCHEMA_UPDATES_OFF")
        ),
        "{reference}"
    );
}

fn load_error(source: &str, pairs: &[(&str, &str)]) -> ConfigError {
    ConfigLoader {
        lookup: environment(pairs),
    }
    .load(source)
    .expect_err("the config should fail")
}
