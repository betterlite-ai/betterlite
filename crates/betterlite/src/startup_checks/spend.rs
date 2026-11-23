use std::sync::Arc;

use async_trait::async_trait;
use betterlite_auth::auth_user_lookup::ports::PortError;
use rust_decimal::Decimal;
use tokio_util::task::TaskTracker;
use tracing::debug;

use crate::startup_checks::presence::DatabasePresence;

// _fetch_global_spend_with_event_coordination: one read of the proxy-wide spend accumulator into the auth cache
#[async_trait]
pub trait GlobalSpendFetch: Send + Sync {
    async fn fetch_global_spend(&self) -> Result<(), PortError>;
}

pub fn warm_global_spend(
    max_budget: Decimal,
    database: DatabasePresence,
    fetch: Arc<dyn GlobalSpendFetch>,
    tasks: &TaskTracker,
) {
    if database == DatabasePresence::Absent || max_budget <= Decimal::ZERO {
        return;
    }
    tasks.spawn(async move {
        if let Err(error) = fetch.fetch_global_spend().await {
            debug!("Global spend cache warm-up at startup skipped or failed: {error}");
        }
    });
}
