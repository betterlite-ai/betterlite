use std::future::Future;
use std::str::Utf8Error;
use std::time::Duration;

use deadpool_postgres::{BuildError, PoolError};
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;
use tokio_postgres::Error as PostgresError;
use uuid::Uuid;

use crate::database_tls::options::ConnectionOptionsError;
use crate::database_tls::translation::{TranslateError, strip_prisma_query_params};
use crate::database_url_policy::url_codec::UrlSplitError;
use crate::migration_deploy::catalog::Migration;
use crate::migration_ledger::baseline::BaselineError;
use crate::migration_ledger::budget::BudgetError;
use crate::migration_ledger::connection::ConnectionError;
use crate::migration_ledger::environment::{
    DEFAULT_SCHEMA, migration_environment, prisma_schema_param,
};
use crate::migration_ledger::inert::InertError;
use crate::migration_ledger::lock::{JitterSource, LockError};
use crate::migration_ledger::recovery::RecoveryError as LedgerRecoveryError;
use crate::schema_contract::SchemaContract;

mod messages;
mod outcome;
mod policy;
mod steps;

mod tests;

use policy::{PassSettings, TokioPause, run_passes};
use steps::{PgSteps, PgStepsInputs};

pub const V2_RESOLVER_ENV_VAR: &str = "USE_V2_MIGRATION_RESOLVER";
pub const DB_PUSH_ENV_VAR: &str = "USE_PRISMA_DB_PUSH";
pub const REPLICA_IDENTITY_ENV_VAR: &str = "LITELLM_SET_REPLICA_IDENTITY_FULL";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetupRequest {
    pub use_v2_resolver: bool,
    pub db_push_requested: bool,
}

pub struct SetupInputs<'config> {
    pub database_url: Option<SecretString>,
    pub direct_url: Option<SecretString>,
    pub catalog: &'config [Migration],
    pub schema_prisma: &'config [u8],
    pub current_build: Option<&'config SchemaContract>,
    pub command_timeout: Duration,
    pub deploy_timeout: Duration,
    pub lock_timeout: Duration,
    pub jitter: &'config dyn JitterSource,
    pub next_id: fn() -> Uuid,
}

#[derive(Debug, Error)]
pub enum RecoveryError {
    #[error("DATABASE_URL is required for v2 migrations")]
    DatabaseUrlRequired,
    #[error(
        "prisma db push is not supported by the native migration resolver. Apply the pinned migrations, or baseline an already-current schema before startup."
    )]
    DbPushRequested,
    #[error("the migration database URL could not be parsed")]
    Url {
        #[source]
        source: UrlSplitError,
    },
    #[error("the migration database URL could not be rewritten for a native connection")]
    Translation {
        #[source]
        source: TranslateError,
    },
    #[error("the migration database URL has no usable native connection options")]
    ConnectionOptions {
        #[source]
        source: ConnectionOptionsError,
    },
    #[error("could not open the migration coordinator connection")]
    Connection {
        #[from]
        source: ConnectionError,
    },
    #[error("could not take a migration deploy connection from its pool")]
    DeployConnection {
        #[from]
        source: PoolError,
    },
    #[error("the migration deploy connection pool could not be built")]
    Pool {
        #[from]
        source: BuildError,
    },
    #[error("the migration database operation failed")]
    Database {
        #[from]
        source: PostgresError,
    },
    #[error("the migration deploy timeout is too large to schedule")]
    DeployTimeout,
    #[error("a migration recovery database query exceeded its {seconds}s command timeout")]
    QueryTimeout { seconds: u64 },
    #[error(transparent)]
    Lock(#[from] LockError),
    #[error(transparent)]
    Ledger(#[from] LedgerRecoveryError),
    #[error(transparent)]
    Inert(#[from] InertError),
    #[error(transparent)]
    Baseline(#[from] BaselineError),
    #[error(transparent)]
    Budget(#[from] BudgetError),
    #[error("{message}")]
    UnverifiedCompletion { message: String },
    #[error("{message}")]
    InsufficientPrivileges {
        message: String,
        #[source]
        source: StepCause,
    },
    #[error("{message}")]
    ManualIntervention {
        message: String,
        #[source]
        source: StepCause,
    },
    #[error("{message}")]
    AttemptsExhausted { message: String },
}

#[derive(Debug, Error)]
pub enum StepCause {
    #[error("database error: {0}")]
    Database(#[from] PostgresError),
    #[error("migration SQL is not valid UTF-8: {0}")]
    Utf8(#[from] Utf8Error),
}

// Mirrors extras/utils.py::str_to_bool: case-insensitive without trimming, and "on" reads false
#[must_use]
pub fn extras_boolean(value: Option<&str>) -> bool {
    let Some(text) = value else {
        return false;
    };
    let lowered = text.to_lowercase();
    lowered == "true" || lowered == "1" || lowered == "t" || lowered == "y" || lowered == "yes"
}

// Mirrors proxy_cli.py::resolve_v2_migration_resolver: the legacy flag wins, and an absent value means v2
#[must_use]
pub fn resolve_v2_migration_resolver(use_legacy_flag: bool, env_value: Option<&str>) -> bool {
    if use_legacy_flag {
        return false;
    }
    env_value.is_none_or(|text| extras_boolean(Some(text)))
}

// The legacy resolver is a recorded deviation: a request for it warns and the native v2 resolver still runs
pub async fn setup_database_v2(
    inputs: &SetupInputs<'_>,
    request: SetupRequest,
) -> Result<(), RecoveryError> {
    if request.db_push_requested {
        return Err(RecoveryError::DbPushRequested);
    }
    if !request.use_v2_resolver {
        tracing::warn!(
            "the legacy v1 migration resolver was requested, but betterlite only ships the native v2 resolver; continuing with the native v2 resolver, which never diffs and forces the schema"
        );
    }
    let database_url = inputs
        .database_url
        .as_ref()
        .filter(|url| !url.expose_secret().is_empty())
        .ok_or(RecoveryError::DatabaseUrlRequired)?;
    let urls = connection_urls(database_url, inputs.direct_url.as_ref())?;
    let steps = PgSteps::new(PgStepsInputs {
        schema: ledger_namespace(database_url.expose_secret())?,
        catalog: inputs.catalog,
        prisma_url: urls.prisma,
        deploy_url: urls.deploy,
        lock_url: urls.lock,
        database_url: database_url.clone(),
        direct_url: inputs.direct_url.clone(),
        schema_prisma: inputs.schema_prisma,
        current_build: inputs.current_build,
        command_timeout: inputs.command_timeout,
        lock_timeout: inputs.lock_timeout,
        jitter: inputs.jitter,
        next_id: inputs.next_id,
    })?;
    let settings = PassSettings {
        deploy_timeout: inputs.deploy_timeout,
        lock_timeout: inputs.lock_timeout,
    };
    run_passes(&steps, settings, inputs.jitter, &TokioPause).await
}

// Mirrors _setup_database_v2: the writer URL's first nonblank schema, else public
fn ledger_namespace(database_url: &str) -> Result<String, RecoveryError> {
    prisma_schema_param(database_url)
        .map_err(|source| RecoveryError::Url { source })
        .map(|schema| schema.unwrap_or_else(|| DEFAULT_SCHEMA.to_owned()))
}

struct ConnectionUrls {
    lock: SecretString,
    deploy: SecretString,
    prisma: SecretString,
}

// The deploy session follows migration_environment; the lock session follows DIRECT_URL or the writer URL
fn connection_urls(
    database_url: &SecretString,
    direct_url: Option<&SecretString>,
) -> Result<ConnectionUrls, RecoveryError> {
    let direct = direct_url.filter(|url| !url.expose_secret().is_empty());
    let prisma = migration_environment(
        Some(database_url.expose_secret()),
        direct.map(SecretString::expose_secret),
    )
    .map_err(|source| RecoveryError::Url { source })?
    .map_or_else(|| database_url.clone(), SecretString::from);
    let lock_source =
        direct.map_or_else(|| database_url.expose_secret(), |url| url.expose_secret());
    Ok(ConnectionUrls {
        lock: stripped(lock_source)?,
        deploy: stripped(prisma.expose_secret())?,
        prisma,
    })
}

fn stripped(url: &str) -> Result<SecretString, RecoveryError> {
    strip_prisma_query_params(url)
        .map(SecretString::from)
        .map_err(|source| RecoveryError::Translation { source })
}

// B051 calls this once after a successful setup; the retry loop never applies it after every attempt
pub async fn replica_identity_if_requested<F, Fut>(enabled: bool, apply: F) -> bool
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = bool>,
{
    if !enabled {
        return false;
    }
    apply().await
}
