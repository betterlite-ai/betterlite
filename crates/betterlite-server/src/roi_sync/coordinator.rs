use async_trait::async_trait;
use betterlite_roi::records::{ROIReport, ROISyncStatus};

// LiteLLM's SyncCoordinator protocol; a failed lease operation is reported as false, the same outcome its raised exceptions have
#[async_trait]
pub trait SyncCoordinator: Send + Sync {
    async fn status(&self) -> Option<ROISyncStatus>;

    async fn acquire(&self, owner: &str, status: &ROISyncStatus, scheduled_interval: f64) -> bool;

    async fn heartbeat(&self, owner: &str, status: &ROISyncStatus) -> bool;

    async fn finish(&self, owner: &str, status: &ROISyncStatus, report: Option<&ROIReport>)
    -> bool;
}
