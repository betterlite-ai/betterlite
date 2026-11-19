use std::sync::Arc;

use async_trait::async_trait;
use betterlite_roi::records::{ROIReport, ROISyncStatus};

use crate::roi_github::transport::Transport;
use crate::roi_sync::coordinator::SyncCoordinator;
use crate::roi_sync::manager::{SyncManager, SyncRun};

use super::ports::{RouteSyncRun, SyncManagerPort, SyncStorePort};

pub(super) struct StoreCoordinator {
    pub(super) store: Arc<dyn SyncStorePort>,
}

#[async_trait]
impl SyncCoordinator for StoreCoordinator {
    async fn status(&self) -> Option<ROISyncStatus> {
        self.store.read_status().await.ok().flatten()
    }

    async fn acquire(&self, owner: &str, status: &ROISyncStatus, scheduled_interval: f64) -> bool {
        self.store
            .acquire_lease(owner, status, scheduled_interval)
            .await
            .unwrap_or(false)
    }

    async fn heartbeat(&self, owner: &str, status: &ROISyncStatus) -> bool {
        self.store
            .heartbeat_lease(owner, status)
            .await
            .unwrap_or(false)
    }

    async fn finish(
        &self,
        owner: &str,
        status: &ROISyncStatus,
        report: Option<&ROIReport>,
    ) -> bool {
        self.store
            .finish_claim(owner, status, report)
            .await
            .unwrap_or(false)
    }
}

// The real manager needs a GitHub transport and a running sync to answer; the route tests fake SyncManagerPort.
#[async_trait]
impl<T> SyncManagerPort for SyncManager<T>
where
    T: Transport + 'static,
{
    #[cfg_attr(test, mutants::skip)]
    fn current_status(&self) -> ROISyncStatus {
        SyncManager::status(self)
    }

    #[cfg_attr(test, mutants::skip)]
    async fn start_run(&self, run: RouteSyncRun) -> bool {
        let RouteSyncRun {
            settings,
            repository,
            spend_reader,
            complete,
            estimator_models,
            store,
            scheduled_interval,
        } = run;
        let coordinator: Option<Arc<dyn SyncCoordinator>> = store
            .map(|port| -> Arc<dyn SyncCoordinator> { Arc::new(StoreCoordinator { store: port }) });
        let sync_run = SyncRun {
            settings,
            repository,
            spend_reader,
            complete,
            github_transport: None,
            estimator_models: Some(estimator_models),
            coordinator,
            scheduled_interval,
        };
        SyncManager::start(self, sync_run).await
    }

    #[cfg_attr(test, mutants::skip)]
    async fn cancel_run(&self) {
        let cancelled = SyncManager::cancel(self).await;
        tracing::debug!(cancelled, "the ROI sync cancel answered");
    }
}
