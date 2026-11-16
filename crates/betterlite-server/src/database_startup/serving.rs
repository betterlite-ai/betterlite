use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use betterlite_config::sections::database_startup::job::{SetupFailure, classify};
use betterlite_config::sections::database_startup::startup::StartupSettings;
use betterlite_db::database_refresh::native::NativePoolError;
use betterlite_db::migration_drift::comparison::SchemaCheck;
use betterlite_db::migration_recovery::RecoveryError;
use betterlite_db::worker_heartbeat::HostnameError;
use thiserror::Error;
use tokio::task::JoinSet;

use super::state::DatabaseState;

pub type StartupFuture<'database, Value> = Pin<Box<dyn Future<Output = Value> + Send + 'database>>;

// The database work the serving policy drives; NativeStartup runs the real migrations behind it.
pub trait StartupDatabase: Send + Sync {
    fn check(&self) -> StartupFuture<'_, SchemaCheck>;
    fn migrate(&self) -> StartupFuture<'_, Result<(), RecoveryError>>;
    fn build_indexes(&self) -> StartupFuture<'_, bool>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartupOutcome {
    NoDatabase,
    UpdatesDisabled(SchemaCheck),
    Migrated,
}

// The owned work a serving startup leaves running: the background request-log index build.
pub struct Startup {
    pub outcome: StartupOutcome,
    pub(crate) tasks: JoinSet<()>,
}

impl Startup {
    pub async fn shutdown(self) {
        let mut tasks = self.tasks;
        while let Some(outcome) = tasks.join_next().await {
            if let Err(error) = outcome
                && !error.is_cancelled()
            {
                tracing::error!("a database startup task stopped with an error: {error}");
            }
        }
    }
}

#[derive(Debug, Error)]
pub enum StartupError {
    #[error("Database migration cannot proceed. {message}")]
    Unrecoverable { message: String },
    #[error(
        "Database setup failed after multiple retries. The proxy cannot start safely. Please check your database connection and migration status."
    )]
    SetupFailed,
    #[error("the database connection pool could not be opened")]
    Pool {
        #[source]
        source: NativePoolError,
    },
    #[error("the proxy worker heartbeat could not start")]
    Heartbeat {
        #[source]
        source: HostnameError,
    },
}

impl StartupError {
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::Unrecoverable { .. } => 2,
            Self::SetupFailed | Self::Pool { .. } | Self::Heartbeat { .. } => 1,
        }
    }

    fn from_recovery(error: &RecoveryError) -> Self {
        match classify(error) {
            SetupFailure::RetriesExhausted => Self::SetupFailed,
            SetupFailure::Unrecoverable => Self::Unrecoverable {
                message: error.to_string(),
            },
        }
    }
}

// proxy_cli.py's serving half: check-only when updates are disabled, else migrate and schedule the
// request-log index build on its own task, so a long build never delays readiness.
pub(crate) struct ServingStartup {
    pub(crate) database: Arc<dyn StartupDatabase>,
    pub(crate) settings: Arc<StartupSettings>,
    pub(crate) state: DatabaseState,
}

impl ServingStartup {
    pub(crate) async fn start(&self) -> Result<Startup, StartupError> {
        if !self.state.is_configured() {
            return Ok(Startup {
                outcome: StartupOutcome::NoDatabase,
                tasks: JoinSet::new(),
            });
        }
        if !self.settings.updates_enabled {
            let check = self.database.check().await;
            return Ok(Startup {
                outcome: StartupOutcome::UpdatesDisabled(check),
                tasks: JoinSet::new(),
            });
        }
        self.database
            .migrate()
            .await
            .map_err(|error| StartupError::from_recovery(&error))?;
        let mut tasks = JoinSet::new();
        let database = Arc::clone(&self.database);
        tasks.spawn(async move {
            // a false result means the build left the indexes for the next run; start continues
            let _built = database.build_indexes().await;
        });
        Ok(Startup {
            outcome: StartupOutcome::Migrated,
            tasks,
        })
    }
}
