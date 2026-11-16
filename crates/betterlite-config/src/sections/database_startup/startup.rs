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

// The server and the offline db-admin command both run this assembly. It sits beside the settings it
// reads because the admin tool may not depend on betterlite-server.
#[derive(Debug, Clone)]
pub struct StartupSettings {
    pub updates_enabled: bool,
    pub database_url: Option<SecretString>,
    pub direct_url: Option<SecretString>,
    pub use_v2_migration_resolver: bool,
    pub use_prisma_db_push: bool,
    pub set_replica_identity_full: bool,
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
            updates_enabled: settings.updates_enabled,
            database_url: endpoints
                .database_url
                .clone()
                .filter(|url| !url.expose_secret().is_empty()),
            direct_url: endpoints
                .direct_url
                .clone()
                .filter(|url| !url.expose_secret().is_empty()),
            use_v2_migration_resolver: recovery.modes.use_v2_migration_resolver,
            use_prisma_db_push: recovery.modes.use_prisma_db_push,
            set_replica_identity_full: recovery.set_replica_identity_full,
            command_timeout: settings.command_timeout,
            lock_timeout: settings.lock_timeout,
            deploy_timeout: settings.deploy_timeout,
            bootstrap_inputs: settings.bootstrap_inputs.clone(),
        }
    }

    fn check(&self) -> MigrationDriftCheck {
        MigrationDriftCheck {
            database_url: self.database_url.clone(),
            direct_url: self.direct_url.clone(),
        }
    }
}

// Mirrors ProxyExtrasDBManager.setup_database after its Node toolchain step: migrations, the invalid
// index repair, then the opt-in REPLICA IDENTITY FULL re-assert, each of the last two best effort.
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
        use_v2_resolver: settings.use_v2_migration_resolver,
        db_push_requested: settings.use_prisma_db_push,
    };
    setup_database_v2(&inputs, request).await?;
    // both best-effort steps log their own failure and never fail the setup
    let _repaired = repair_invalid_indexes(&settings.check(), REPAIR_LOCK_TIMEOUT).await;
    let _identity = replica_identity_if_requested(settings.set_replica_identity_full, || {
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

// Mirrors ProxyExtrasDBManager.build_request_log_indexes: no DATABASE_URL succeeds without a build,
// DIRECT_URL carries the connection and DATABASE_URL names the schema, and a failure is never raised.
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

// Mirrors the disabled-updates branch of proxy startup: the comparison is warning-only, so it can never
// fail startup. The expected contract belongs to the drift service, so an absent one warns and continues.
pub async fn check_schema_diff(settings: &StartupSettings) -> SchemaCheck {
    check_schema(
        &settings.check(),
        !settings.updates_enabled,
        None,
        &settings
            .database_url
            .as_ref()
            .map_or_else(|| DEFAULT_SCHEMA.to_owned(), |url| schema_of(url)),
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
