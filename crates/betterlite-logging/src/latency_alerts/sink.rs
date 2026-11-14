use async_trait::async_trait;

use crate::alert_dispatch::Alert;

#[async_trait]
pub trait AlertSink: Send + Sync {
    async fn send_alert(&self, alert: Alert<'_>);
}
