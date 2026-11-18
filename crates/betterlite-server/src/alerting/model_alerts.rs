use std::error::Error as StdError;

use async_trait::async_trait;
use betterlite_logging::alert_dispatch::Alert;

pub mod fallback_stats;
pub mod key_event;
pub mod model_added;
pub mod model_deprecation;

#[async_trait]
pub trait ModelAlertSender: Send + Sync {
    type Error: StdError + Send + Sync + 'static;

    async fn send_alert(&self, alert: Alert<'_>) -> Result<(), Self::Error>;
}

mod tests;
