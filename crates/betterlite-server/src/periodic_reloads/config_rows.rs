use std::error::Error;
use std::future::Future;

use betterlite_cache::backend::memory::MemoryCacheOps;
use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::reload_schedule::ReloadSchedule;
use betterlite_db::reload_schedule::evictor::ConfigParamEvictor;
use betterlite_db::reload_schedule::store::ReloadScheduleError;
use betterlite_db::reload_schedule::store::ScheduleDeadlines;
use betterlite_db::reload_schedule::store::read_reload_schedule;
use betterlite_db::reload_schedule::store::record_reload_run;
use betterlite_db::resource_repository::config::ConfigError;
use betterlite_db::resource_repository::config::ConfigRepository;
use betterlite_db::table_ops::rows::ConfigParamRow;
use chrono::{DateTime, Utc};

use crate::config_param_cache::ConfigParamCache;
use crate::config_param_cache::ConfigParamCacheError;
use crate::config_param_cache::store::ConfigParamStore;

const BETA_HEADERS_RELOAD_PARAM_NAME: &str = "anthropic_beta_headers_reload_config";

// the two LiteLLM_Config rows the periodic checks touch, behind one port so a test serves them in memory
pub trait ReloadConfigRows: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn read_schedule(
        &self,
    ) -> impl Future<Output = Result<Option<ReloadSchedule>, Self::Error>> + Send;

    fn record_run(
        &self,
        ran_at: DateTime<Utc>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    fn read_beta_headers(
        &self,
    ) -> impl Future<Output = Result<Option<ConfigParamRow>, Self::Error>> + Send;

    // the check always stores force_reload false and the interval it just read, as LiteLLM's upsert does
    fn write_beta_headers(
        &self,
        value: JsonData,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    fn evict_beta_headers(&self) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigRowsError<ClientError>
where
    ClientError: Error + Send + Sync + 'static,
{
    #[error("the model cost map reload schedule could not be read: {source}")]
    Schedule {
        #[source]
        source: ReloadScheduleError,
    },
    #[error("the model cost map reload run could not be recorded: {source}")]
    Record {
        #[source]
        source: ReloadScheduleError,
    },
    #[error("the anthropic beta headers reload config could not be read: {source}")]
    BetaRead {
        #[source]
        source: ConfigParamCacheError<ClientError>,
    },
    #[error("the anthropic beta headers reload config could not be written: {source}")]
    BetaWrite {
        #[source]
        source: ConfigError,
    },
    #[error("the anthropic beta headers reload config could not be evicted: {source}")]
    BetaEvict {
        #[source]
        source: ConfigParamCacheError,
    },
}

pub struct DatabaseReloadConfigRows<'rows, 'cache, Memory, Store>
where
    Memory: MemoryCacheOps,
{
    pub client: Option<PrismaClient<'rows>>,
    pub evictor: &'rows dyn ConfigParamEvictor,
    pub deadlines: ScheduleDeadlines,
    pub cache: &'rows ConfigParamCache<'cache, Memory>,
    pub store: &'rows Store,
}

impl<Memory, Store> ReloadConfigRows for DatabaseReloadConfigRows<'_, '_, Memory, Store>
where
    Memory: MemoryCacheOps,
    Store: ConfigParamStore,
{
    type Error = ConfigRowsError<Store::ClientError>;

    async fn read_schedule(&self) -> Result<Option<ReloadSchedule>, Self::Error> {
        read_reload_schedule(self.client, self.deadlines)
            .await
            .map_err(|source| ConfigRowsError::Schedule { source })
    }

    async fn record_run(&self, ran_at: DateTime<Utc>) -> Result<(), Self::Error> {
        record_reload_run(self.client, self.evictor, ran_at, self.deadlines)
            .await
            .map_err(|source| ConfigRowsError::Record { source })
    }

    async fn read_beta_headers(&self) -> Result<Option<ConfigParamRow>, Self::Error> {
        self.cache
            .get(self.store, BETA_HEADERS_RELOAD_PARAM_NAME)
            .await
            .map_err(|source| ConfigRowsError::BetaRead { source })
    }

    async fn write_beta_headers(&self, value: JsonData) -> Result<(), Self::Error> {
        // LiteLLM's ConfigRepository(prisma_client) keeps its default read routing; the upsert itself writes to the primary
        let repository = ConfigRepository::new(self.client, false);
        repository
            .set_param(BETA_HEADERS_RELOAD_PARAM_NAME, value)
            .await
            .map_err(|source| ConfigRowsError::BetaWrite { source })?;
        Ok(())
    }

    async fn evict_beta_headers(&self) -> Result<(), Self::Error> {
        self.cache
            .evict(BETA_HEADERS_RELOAD_PARAM_NAME)
            .await
            .map_err(|source| ConfigRowsError::BetaEvict { source })
    }
}
