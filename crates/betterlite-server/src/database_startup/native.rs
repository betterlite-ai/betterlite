use std::sync::Arc;

use betterlite_config::sections::database_startup::packaged;
use betterlite_config::sections::database_startup::startup::{
    StartupSettings, build_request_log_indexes, check_schema_diff, setup_database,
};
use betterlite_db::database_refresh::handle::{PoolHandle as _, PoolProvider as _};
use betterlite_db::database_refresh::native::{NativePool, NativePoolProvider};
use betterlite_db::migration_deploy::catalog::Migration;
use betterlite_db::migration_drift::comparison::SchemaCheck;
use betterlite_db::migration_ledger::lock::{JitterSource, LockError, SystemJitter};
use betterlite_db::migration_recovery::RecoveryError;
use betterlite_db::worker_heartbeat::{
    PostgresDatabase, ProxyWorkerHeartbeat, SystemWorkerIdentity,
};

use super::heartbeat::HeartbeatTask;
use super::refresh::CredentialRefresh;
use super::serving::{
    ServingStartup, Startup, StartupDatabase, StartupError, StartupFuture, StartupOutcome,
};
use super::state::DatabaseState;

// The startup policy's database work over the packaged migrations and the native pools.
pub struct NativeStartup {
    settings: StartupSettings,
    catalog: Vec<Migration>,
    schema_prisma: &'static [u8],
    jitter: Box<dyn JitterSource>,
}

impl NativeStartup {
    #[must_use]
    pub fn new(
        settings: StartupSettings,
        catalog: Vec<Migration>,
        schema_prisma: &'static [u8],
        jitter: Box<dyn JitterSource>,
    ) -> Self {
        Self {
            settings,
            catalog,
            schema_prisma,
            jitter,
        }
    }

    pub fn packaged(settings: StartupSettings) -> Result<Self, LockError> {
        Ok(Self::new(
            settings,
            packaged::migrations(),
            packaged::SCHEMA_PRISMA,
            Box::new(SystemJitter::try_new()?),
        ))
    }
}

impl StartupDatabase for NativeStartup {
    fn check(&self) -> StartupFuture<'_, SchemaCheck> {
        Box::pin(check_schema_diff(&self.settings))
    }

    fn migrate(&self) -> StartupFuture<'_, Result<(), RecoveryError>> {
        Box::pin(setup_database(
            &self.settings,
            &self.catalog,
            self.schema_prisma,
            self.jitter.as_ref(),
        ))
    }

    fn build_indexes(&self) -> StartupFuture<'_, bool> {
        Box::pin(build_request_log_indexes(&self.settings))
    }
}

// The serving process's database startup: one policy run, then the native pool, the worker
// heartbeat and the injected credential-refresh duty. Shutdown drains every one of them.
pub struct NativeDatabase<Startup = NativeStartup> {
    startup: Arc<Startup>,
    settings: Arc<StartupSettings>,
    state: DatabaseState,
    refresh: Option<Arc<dyn CredentialRefresh>>,
}

impl NativeDatabase<NativeStartup> {
    pub fn packaged(
        settings: StartupSettings,
        refresh: Option<Arc<dyn CredentialRefresh>>,
    ) -> Result<Self, LockError> {
        let startup = NativeStartup::packaged(settings.clone())?;
        Ok(Self::with_startup(Arc::new(startup), settings, refresh))
    }
}

impl<Startup> NativeDatabase<Startup>
where
    Startup: StartupDatabase + 'static,
{
    #[must_use]
    pub fn with_startup(
        startup: Arc<Startup>,
        settings: StartupSettings,
        refresh: Option<Arc<dyn CredentialRefresh>>,
    ) -> Self {
        let state = DatabaseState::from_settings(&settings);
        Self {
            startup,
            settings: Arc::new(settings),
            state,
            refresh,
        }
    }

    pub async fn start(&self) -> Result<Running, StartupError> {
        let serving = ServingStartup {
            database: Arc::clone(&self.startup),
            settings: Arc::clone(&self.settings),
            state: self.state,
        };
        let startup = serving.start().await?;
        let Some(url) = self.settings.database_url.as_ref() else {
            return Ok(Running::without_database(self.state, startup));
        };
        let pool = match NativePoolProvider.build(url, None) {
            Ok(pool) => Arc::new(pool),
            Err(source) => {
                startup.shutdown().await;
                return Err(StartupError::Pool { source });
            }
        };
        let client = match pool.session().await {
            Ok(client) => client,
            Err(source) => {
                startup.shutdown().await;
                pool.close();
                return Err(StartupError::Pool { source });
            }
        };
        let heartbeat = match ProxyWorkerHeartbeat::new(None, &SystemWorkerIdentity) {
            Ok(heartbeat) => heartbeat,
            Err(source) => {
                startup.shutdown().await;
                pool.close();
                return Err(StartupError::Heartbeat { source });
            }
        };
        let heartbeat =
            HeartbeatTask::start(Arc::new(heartbeat), Arc::new(PostgresDatabase { client })).await;
        if let Some(refresh) = &self.refresh {
            refresh.start();
        }
        Ok(Running {
            state: self.state,
            startup: Some(startup),
            heartbeat: Some(heartbeat),
            refresh: self.refresh.clone(),
            pool: Some(pool),
        })
    }
}

// What a finished startup owns. B4448 reads the state; shutdown deregisters, drains and closes.
pub struct Running {
    pub(crate) state: DatabaseState,
    pub(crate) startup: Option<Startup>,
    pub(crate) heartbeat: Option<HeartbeatTask>,
    pub(crate) refresh: Option<Arc<dyn CredentialRefresh>>,
    pub(crate) pool: Option<Arc<NativePool>>,
}

impl Running {
    fn without_database(state: DatabaseState, startup: Startup) -> Self {
        Self {
            state,
            startup: Some(startup),
            heartbeat: None,
            refresh: None,
            pool: None,
        }
    }

    #[must_use]
    pub const fn state(&self) -> DatabaseState {
        self.state
    }

    #[must_use]
    pub fn outcome(&self) -> Option<&StartupOutcome> {
        self.startup.as_ref().map(|startup| &startup.outcome)
    }

    #[must_use]
    pub fn pool(&self) -> Option<&Arc<NativePool>> {
        self.pool.as_ref()
    }

    // LiteLLM's shutdown order: deregister this worker, drain owned work, then close the pool
    pub async fn shutdown(self) {
        if let Some(heartbeat) = self.heartbeat
            && let Err(error) = heartbeat.stop().await
            && !error.is_cancelled()
        {
            tracing::error!("the proxy worker heartbeat task stopped with an error: {error}");
        }
        if let Some(refresh) = &self.refresh
            && let Err(error) = refresh.stop().await
        {
            tracing::error!("the database credential refresh task stopped with an error: {error}");
        }
        if let Some(startup) = self.startup {
            startup.shutdown().await;
        }
        if let Some(pool) = &self.pool {
            pool.close();
        }
    }
}
