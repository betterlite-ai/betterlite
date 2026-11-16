#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::sync::Arc;
use std::time::Duration;

use secrecy::ExposeSecret as _;
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
    assert_eq!(settings.deploy_timeout, Duration::from_secs(1200));
    assert_eq!(settings.bootstrap_inputs, IGNORED_BOOTSTRAP_INPUTS.to_vec());

    let startup = StartupSettings::from_config(&loaded);
    assert!(startup.updates_enabled);
    assert_eq!(
        startup.database_url.as_ref().map(|url| url.expose_secret()),
        Some("postgresql://writer/db?schema=tenant")
    );
    assert_eq!(
        startup.direct_url.as_ref().map(|url| url.expose_secret()),
        Some("postgresql://direct/db")
    );
    assert!(!startup.use_v2_migration_resolver);
    assert!(startup.use_prisma_db_push);
    assert!(startup.set_replica_identity_full);
    assert_eq!(startup.command_timeout, Duration::from_secs(90));
    assert_eq!(startup.lock_timeout, Duration::from_secs(450));
    assert_eq!(startup.deploy_timeout, Duration::from_secs(1200));
    assert_eq!(startup.bootstrap_inputs, IGNORED_BOOTSTRAP_INPUTS.to_vec());
}

#[test]
fn defaults_and_precedence() {
    let defaults = config("general_settings: {}\n", &[])
        .sections
        .database_startup;
    assert!(defaults.updates_enabled);
    assert_eq!(defaults.command_timeout, Duration::from_secs(60));
    assert_eq!(defaults.lock_timeout, Duration::from_secs(600));
    assert_eq!(defaults.deploy_timeout, Duration::from_secs(600));
    assert!(defaults.bootstrap_inputs.is_empty());

    let empty_environment = environment(&[("DISABLE_SCHEMA_UPDATE", "")]);
    let loaded = ConfigLoader {
        lookup: empty_environment,
    }
    .load("general_settings: {}\n")
    .expect("an empty environment value counts as unset")
    .sections
    .database_startup;
    assert!(loaded.updates_enabled);

    let settings_overrides_environment = [
        (
            "general_settings:\n  disable_prisma_schema_update: false\n",
            &[("DISABLE_SCHEMA_UPDATE", "true")][..],
            true,
        ),
        (
            "general_settings:\n  disable_prisma_schema_update: true\n",
            &[("DISABLE_SCHEMA_UPDATE", "false")][..],
            false,
        ),
        (
            "general_settings:\n  disable_prisma_schema_update: null\n",
            &[("DISABLE_SCHEMA_UPDATE", "yes")][..],
            false,
        ),
        (
            "general_settings:\n  disable_prisma_schema_update: \"TRUE\"\n",
            &[][..],
            false,
        ),
        (
            "general_settings:\n  disable_prisma_schema_update: \"os.environ/SCHEMA_UPDATES_OFF\"\n",
            &[("SCHEMA_UPDATES_OFF", "1")][..],
            false,
        ),
        (
            "general_settings:\n  disable_prisma_schema_update: \"os.environ/SCHEMA_UPDATES_OFF\"\n",
            &[][..],
            true,
        ),
        (
            "general_settings:\n  disable_prisma_schema_update: \"os.environ/SCHEMA_UPDATES_OFF\"\n",
            &[("DISABLE_SCHEMA_UPDATE", "true")][..],
            false,
        ),
        (
            "general_settings:\n  disable_prisma_schema_update: \"yes\"\n",
            &[][..],
            true,
        ),
    ];
    for (source, pairs, expected) in settings_overrides_environment {
        let loaded = config(source, pairs).sections.database_startup;
        assert_eq!(loaded.updates_enabled, expected, "{source} with {pairs:?}");
    }

    let inherited = config(
        "general_settings: {}\n",
        &[("LITELLM_PRISMA_COMMAND_TIMEOUT", "900")],
    )
    .sections
    .database_startup;
    assert_eq!(inherited.command_timeout, Duration::from_secs(900));
    assert_eq!(inherited.deploy_timeout, Duration::from_secs(900));

    let pinned_deploy = config(
        "general_settings: {}\n",
        &[
            ("LITELLM_PRISMA_COMMAND_TIMEOUT", "900"),
            ("LITELLM_PRISMA_MIGRATE_DEPLOY_TIMEOUT", "1200"),
        ],
    )
    .sections
    .database_startup;
    assert_eq!(pinned_deploy.deploy_timeout, Duration::from_secs(1200));

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
    assert_eq!(empty_deploy.deploy_timeout, Duration::from_secs(900));

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
    assert_eq!(fallback_warnings(&log), 1);
}

#[test]
fn invalid_value_fallback() {
    for invalid in ["abc", "0", "-5", "inf", "NaN"] {
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
        assert_eq!(fallback_warnings(&log), 3, "warnings for {invalid}");
        for name in [
            "LITELLM_PRISMA_COMMAND_TIMEOUT",
            "LITELLM_MIGRATION_LOCK_TIMEOUT",
            "LITELLM_PRISMA_MIGRATE_DEPLOY_TIMEOUT",
        ] {
            assert!(log.contains(name), "the warning names {name}: {log}");
        }
    }

    let (inherited, log) = logged(|| {
        config(
            "general_settings: {}\n",
            &[("LITELLM_PRISMA_COMMAND_TIMEOUT", "abc")],
        )
        .sections
        .database_startup
    });
    assert_eq!(inherited.deploy_timeout, Duration::from_secs(600));
    assert_eq!(fallback_warnings(&log), 2);

    let error = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("general_settings:\n  disable_prisma_schema_update: [true]\n")
    .err()
    .expect("a list is not a bool or a string");
    assert!(
        matches!(
            &error,
            ConfigError::InvalidScalar { path, .. }
                if path.as_str() == "general_settings.disable_prisma_schema_update"
        ),
        "{error}"
    );

    let error = ConfigLoader {
        lookup: environment(&[("DISABLE_SCHEMA_UPDATE", "maybe")]),
    }
    .load("general_settings: {}\n")
    .err()
    .expect("unrecognized environment text names its variable");
    assert!(
        matches!(
            &error,
            ConfigError::Environment { source } if source.to_string().contains("DISABLE_SCHEMA_UPDATE")
        ),
        "{error}"
    );

    let error = ConfigLoader {
        lookup: environment(&[("SCHEMA_UPDATES_OFF", "maybe")]),
    }
    .load("general_settings:\n  disable_prisma_schema_update: os.environ/SCHEMA_UPDATES_OFF\n")
    .err()
    .expect("unrecognized referenced text names its variable");
    assert!(
        matches!(
            &error,
            ConfigError::Environment { source } if source.to_string().contains("SCHEMA_UPDATES_OFF")
        ),
        "{error}"
    );
}
