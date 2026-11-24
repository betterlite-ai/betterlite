use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use betterlite_core::api::anthropic_beta_config::matrix::BetaMatrix;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::reload_schedule::ReloadSchedule;
use betterlite_db::reload_schedule::store::ReloadScheduleError;
use betterlite_db::resource_repository::config::ConfigError;
use betterlite_models::reload::ReloadOutcome;
use betterlite_models::source::CatalogSnapshot;
use chrono::{DateTime, Utc};

pub type CatalogReloadFuture<'call> = Pin<Box<dyn Future<Output = ReloadOutcome> + Send + 'call>>;

pub type BetaReloadFuture<'call> = Pin<Box<dyn Future<Output = Arc<BetaMatrix>> + Send + 'call>>;

pub type StoreFuture<'call, Value> =
    Pin<Box<dyn Future<Output = Result<Value, ReloadStoreError>> + Send + 'call>>;

// Both LiteLLM_Config writers: the B4145 schedule helpers and B725's config repository
#[derive(Debug, thiserror::Error)]
pub enum ReloadStoreError {
    #[error(transparent)]
    Schedule(#[from] ReloadScheduleError),
    #[error(transparent)]
    Config(#[from] ConfigError),
}

// LiteLLM's refetch and get_model_cost_map_* singleton: the reload adopts, the current snapshot answers the read routes
pub trait CatalogReload: Send + Sync {
    fn current(&self) -> Arc<CatalogSnapshot>;
    fn reload(&self) -> CatalogReloadFuture<'_>;
}

// B959's reload_beta_headers_config, which never fails and falls back to the bundled config
pub trait BetaHeadersReload: Send + Sync {
    fn reload(&self) -> BetaReloadFuture<'_>;
}

// The LiteLLM_Config rows the reload routes read and write; None for read_beta_config means no row or a null param_value, and every beta write is followed by invalidate_beta_config
pub trait ReloadConfigStore: Send + Sync {
    fn read_schedule(&self) -> StoreFuture<'_, Option<ReloadSchedule>>;
    fn write_interval(&self, interval_hours: u32) -> StoreFuture<'_, ()>;
    fn clear_interval(&self) -> StoreFuture<'_, ()>;
    fn record_manual_reload(&self, ran_at: DateTime<Utc>) -> StoreFuture<'_, u32>;
    fn read_beta_config(&self) -> StoreFuture<'_, Option<JsonData>>;
    fn write_beta_config(&self, value: JsonData) -> StoreFuture<'_, ()>;
    fn delete_beta_config(&self) -> StoreFuture<'_, ()>;
    fn invalidate_beta_config(&self) -> StoreFuture<'_, ()>;
}
