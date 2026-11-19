use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_roi::estimator::models::EstimatorModel;
use betterlite_roi::records::{ROIReport, ROISpendRecord, ROISyncStatus};
use betterlite_roi::settings::ROISettings;
use chrono::NaiveDate;

use crate::roi_connections::error::RoiConnectionsError;
use crate::roi_estimator::CompletionCaller;
use crate::roi_settings::repository::SettingsRepository;

use super::error::RoiRoutesError;

pub type SpendFuture<'read> =
    Pin<Box<dyn Future<Output = Result<Vec<ROISpendRecord>, DbError>> + Send + 'read>>;

pub trait SpendReads: Send + Sync {
    fn read_spend(&self, start: NaiveDate, end: NaiveDate) -> SpendFuture<'_>;
}

pub type SpendReaderFactory = Arc<dyn Fn() -> Arc<dyn SpendReads> + Send + Sync>;

pub type CompletionFactory =
    Arc<dyn Fn(&ROISettings) -> Result<CompletionCaller, RoiConnectionsError> + Send + Sync>;

#[async_trait]
pub trait SyncStorePort: Send + Sync {
    async fn read_status(&self) -> Result<Option<ROISyncStatus>, RoiRoutesError>;

    async fn cancel_sync(&self) -> Result<(), RoiRoutesError>;

    async fn acquire_lease(
        &self,
        owner: &str,
        status: &ROISyncStatus,
        scheduled_interval: f64,
    ) -> Result<bool, RoiRoutesError>;

    async fn heartbeat_lease(
        &self,
        owner: &str,
        status: &ROISyncStatus,
    ) -> Result<bool, RoiRoutesError>;

    async fn finish_claim(
        &self,
        owner: &str,
        status: &ROISyncStatus,
        report: Option<&ROIReport>,
    ) -> Result<bool, RoiRoutesError>;

    async fn clear_stored_report(&self) -> Result<(), RoiRoutesError>;
}

#[async_trait]
pub trait SyncStoreSource: Send + Sync {
    async fn store(&self) -> Result<Arc<dyn SyncStorePort>, RoiRoutesError>;
}

pub struct RouteSyncRun {
    pub settings: ROISettings,
    pub repository: Arc<dyn SettingsRepository>,
    pub spend_reader: Arc<dyn SpendReads>,
    pub complete: CompletionCaller,
    pub estimator_models: Vec<EstimatorModel>,
    pub store: Option<Arc<dyn SyncStorePort>>,
    pub scheduled_interval: f64,
}

#[async_trait]
pub trait SyncManagerPort: Send + Sync {
    fn current_status(&self) -> ROISyncStatus;

    async fn start_run(&self, run: RouteSyncRun) -> bool;

    async fn cancel_active_run(&self);
}
