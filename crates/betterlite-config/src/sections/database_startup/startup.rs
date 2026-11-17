use std::time::Duration;

use betterlite_db::database_tls::translation::strip_prisma_query_params;
use betterlite_db::migration_deploy::catalog::Migration;
use betterlite_db::migration_drift::MigrationDriftCheck;
use betterlite_db::migration_drift::comparison::{SchemaCheck, check_schema};
use betterlite_db::migration_drift::index_repair::repair_invalid_indexes;
use betterlite_db::migration_ledger::environment::{DEFAULT_SCHEMA, prisma_schema_param};
use betterlite_db::migration_ledger::lock::JitterSource;
use betterlite_db::migration_recovery::{
    RecoveryError, SetupInputs, SetupRequest, replica_identity_if_requested, setup_database_v2,
};
use betterlite_db::request_log_index_catalog::definitions::REQUEST_LOG_INDEXES;
use betterlite_db::request_log_indexes::ensure::ensure_request_log_indexes;
use betterlite_db::request_log_indexes::replica_identity::apply_replica_identity_full;
use secrecy::{ExposeSecret as _, SecretString};
use uuid::Uuid;

use crate::types::Config;

// repair_invalid_indexes' Python default, which setup_database never overrides
const REPAIR_LOCK_TIMEOUT: &str = "30s";

// clippy's struct_excessive_bools allows two bools per struct, and these four switches are independent
#[derive(Debug, Clone, Copy)]
pub struct StartupSwitch(pub bool);

// the assembly lives in the config crate because the db-admin tool may not depend on betterlite-server
#[derive(Debug, Clone)]
pub struct StartupSettings {
    pub updates_enabled: StartupSwitch,
    pub database_url: Option<SecretString>,
    pub direct_url: Option<SecretString>,
    pub use_v2_migration_resolver: StartupSwitch,
    pub use_prisma_db_push: StartupSwitch,
    pub set_replica_identity_full: StartupSwitch,
    pub command_timeout: Duration,
    pub lock_timeout: Duration,
    pub deploy_timeout: Duration,
    pub bootstrap_inputs: Vec<&'static str>,
}

impl StartupSettings {
    pub fn from_config(config: &Config) -> Self {
        let settings = &config.sections.database_startup;
        let endpoints = &config.sections.database_urls.endpoints;
        let recovery = &config.sections.migration_recovery;
        if !settings.bootstrap_inputs.is_empty() {
            let names = settings.bootstrap_inputs.join(", ");
            tracing::warn!(
                "the Prisma bootstrap inputs {names} are accepted but have no effect: betterlite runs the migrations natively and needs no Node toolchain"
            );
        }
        Self {
            updates_enabled: StartupSwitch(settings.updates_enabled),
            database_url: endpoints
                .database_url
                .clone()
                .filter(|url| !url.expose_secret().is_empty()),
            direct_url: endpoints
                .direct_url
                .clone()
                .filter(|url| !url.expose_secret().is_empty()),
            use_v2_migration_resolver: StartupSwitch(recovery.modes.use_v2_migration_resolver),
            use_prisma_db_push: StartupSwitch(recovery.modes.use_prisma_db_push),
            set_replica_identity_full: StartupSwitch(recovery.set_replica_identity_full),
            command_timeout: settings.command_timeout,
            lock_timeout: settings.lock_timeout,
            deploy_timeout: settings.deploy_timeout,
            bootstrap_inputs: settings.bootstrap_inputs.clone(),
        }
    }
}

pub async fn setup_database(
    settings: &StartupSettings,
    catalog: &[Migration],
    schema_prisma: &[u8],
    jitter: &dyn JitterSource,
) -> Result<(), RecoveryError> {
    let inputs = SetupInputs {
        database_url: settings.database_url.clone(),
        direct_url: settings.direct_url.clone(),
        catalog,
        schema_prisma,
        current_build: None,
        command_timeout: settings.command_timeout,
        deploy_timeout: settings.deploy_timeout,
        lock_timeout: settings.lock_timeout,
        jitter,
        next_id: Uuid::new_v4,
    };
    let request = SetupRequest {
        use_v2_resolver: settings.use_v2_migration_resolver.0,
        db_push_requested: settings.use_prisma_db_push.0,
    };
    setup_database_v2(&inputs, request).await?;
    // both best-effort steps log their own failure and never fail the setup
    let drift = MigrationDriftCheck {
        database_url: settings.database_url.clone(),
        direct_url: settings.direct_url.clone(),
    };
    let _repaired = repair_invalid_indexes(&drift, REPAIR_LOCK_TIMEOUT).await;
    let _identity = replica_identity_if_requested(settings.set_replica_identity_full.0, || {
        apply_identity(settings)
    })
    .await;
    Ok(())
}

async fn apply_identity(settings: &StartupSettings) -> bool {
    match &settings.database_url {
        Some(url) => apply_replica_identity_full(url, settings.command_timeout).await,
        None => false,
    }
}

// a failed build only warns: the next boot or migration job builds the indexes again
pub async fn build_request_log_indexes(settings: &StartupSettings) -> bool {
    let Some(database_url) = &settings.database_url else {
        return true;
    };
    let target = settings.direct_url.as_ref().unwrap_or(database_url);
    let Ok(direct) = strip_prisma_query_params(target.expose_secret()) else {
        tracing::warn!(
            "the database URL could not be normalized for the request-log index build, leaving the indexes for the next index build"
        );
        return false;
    };
    let schema = schema_of(database_url);
    ensure_request_log_indexes(&SecretString::from(direct), &schema, &REQUEST_LOG_INDEXES).await
}

// the comparison is warning-only, so a disabled-updates startup never fails on it
pub async fn check_schema_diff(settings: &StartupSettings) -> SchemaCheck {
    check_schema(
        &MigrationDriftCheck {
            database_url: settings.database_url.clone(),
            direct_url: settings.direct_url.clone(),
        },
        !settings.updates_enabled.0,
        None,
        &settings
            .database_url
            .as_ref()
            .map_or_else(|| DEFAULT_SCHEMA.to_owned(), schema_of),
        settings.command_timeout,
    )
    .await
}

fn schema_of(url: &SecretString) -> String {
    prisma_schema_param(url.expose_secret())
        .ok()
        .flatten()
        .unwrap_or_else(|| DEFAULT_SCHEMA.to_owned())
}
