use std::error::Error as StdError;

use async_trait::async_trait;
use betterlite_config::sections::alert_settings::types::AlertType;
use betterlite_logging::alert_dispatch::{Alert, AlertLevel};
use indexmap::IndexMap;

use super::ModelAlertSender;

#[async_trait]
pub trait FallbackMetricSource: Send + Sync {
    type Error: StdError + Send + Sync + 'static;

    async fn fallback_metric(&self) -> Result<String, Self::Error>;
}

pub struct FallbackStatsAlerts<Source, Sender> {
    pub source: Source,
    pub sender: Sender,
}

impl<Source, Sender> FallbackStatsAlerts<Source, Sender>
where
    Source: FallbackMetricSource,
    Sender: ModelAlertSender,
{
    pub async fn send_fallback_stats(&self) {
        let metric = match self.source.fallback_metric().await {
            Ok(metric) => metric,
            Err(error) => {
                tracing::error!("Error sending weekly spend report {error}");
                return;
            }
        };
        let message = format!("*Fallback Statistics:*\n{metric}");
        let metadata = IndexMap::new();
        let alert = Alert {
            message: &message,
            level: AlertLevel::Low,
            alert_type: AlertType::FallbackReports,
            alerting_metadata: &metadata,
            user_info: None,
            request_model: None,
            api_base: None,
            kwargs: &metadata,
        };
        if let Err(error) = self.sender.send_alert(alert).await {
            tracing::error!("Error sending weekly spend report {error}");
        }
    }
}

#[cfg(test)]
mod tests;
