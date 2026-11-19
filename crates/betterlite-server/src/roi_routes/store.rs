use std::sync::Arc;

use async_trait::async_trait;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::sync_store::{FinishClaim, SyncStore};
use betterlite_roi::records::{ROIReport, ROISyncStatus};

use super::error::RoiRoutesError;
use super::ports::{SyncStorePort, SyncStoreSource};

pub struct DatabaseSyncStoreSource {
    pub store: Arc<PostgresStore>,
}

#[async_trait]
impl SyncStoreSource for DatabaseSyncStoreSource {
    async fn store(&self) -> Result<Arc<dyn SyncStorePort>, RoiRoutesError> {
        let Some(pool) = self.store.pool.as_ref() else {
            return Err(RoiRoutesError::DbNotConnected);
        };
        let port: Arc<dyn SyncStorePort> = Arc::new(SyncStore::open(pool).await?);
        Ok(port)
    }
}

// These delegate to the database store, whose statements need a live connection; the route tests fake SyncStorePort.
#[async_trait]
impl SyncStorePort for SyncStore {
    #[cfg_attr(test, mutants::skip)]
    async fn read_status(&self) -> Result<Option<ROISyncStatus>, RoiRoutesError> {
        Ok(Self::status(self).await?)
    }

    #[cfg_attr(test, mutants::skip)]
    async fn cancel_sync(&self) -> Result<(), RoiRoutesError> {
        Ok(Self::cancel(self).await?)
    }

    #[cfg_attr(test, mutants::skip)]
    async fn acquire_lease(
        &self,
        owner: &str,
        status: &ROISyncStatus,
        scheduled_interval: f64,
    ) -> Result<bool, RoiRoutesError> {
        Ok(Self::acquire(self, owner, status, scheduled_interval).await?)
    }

    #[cfg_attr(test, mutants::skip)]
    async fn heartbeat_lease(
        &self,
        owner: &str,
        status: &ROISyncStatus,
    ) -> Result<bool, RoiRoutesError> {
        Ok(Self::heartbeat(self, owner, status).await?)
    }

    #[cfg_attr(test, mutants::skip)]
    async fn finish_claim(
        &self,
        owner: &str,
        status: &ROISyncStatus,
        report: Option<&ROIReport>,
    ) -> Result<bool, RoiRoutesError> {
        let claim = FinishClaim {
            owner,
            status,
            report,
        };
        Ok(Self::finish(self, claim).await?)
    }

    #[cfg_attr(test, mutants::skip)]
    async fn clear_stored_report(&self) -> Result<(), RoiRoutesError> {
        Ok(Self::clear_report(self).await?)
    }
}
