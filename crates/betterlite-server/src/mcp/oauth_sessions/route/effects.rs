use std::convert::Infallible;

use async_trait::async_trait;
use betterlite_config::sections::alert_settings::types::AlertType;
use betterlite_logging::otel::service::management::ManagementPayload;
use betterlite_logging::otel::v2_callback::live_span::LiveSpan;

use crate::alerting::model_alerts::key_event::VirtualKeyEvent;
use crate::management_effects::eviction::ManagementCacheEviction;
use crate::management_payload::alerts::ManagementAlertTransport;
use crate::management_payload::otel_span::ManagementSpanTransport;

// the one-place effects of this route: no alert, no span transport and no cached key to evict
pub(super) struct NoManagementEffects;

#[async_trait]
impl ManagementAlertTransport for NoManagementEffects {
    type Error = Infallible;

    async fn send_management_event_alert(
        &self,
        _key_event: &VirtualKeyEvent,
        _alert_type: AlertType,
        _event_name: &str,
    ) -> Result<(), Infallible> {
        Ok(())
    }
}

#[async_trait]
impl ManagementSpanTransport for NoManagementEffects {
    type Error = Infallible;

    async fn success_hook(
        &self,
        _payload: &ManagementPayload,
        _parent_otel_span: Option<&LiveSpan>,
    ) -> Result<(), Infallible> {
        Ok(())
    }

    async fn failure_hook(
        &self,
        _payload: &ManagementPayload,
        _parent_otel_span: Option<&LiveSpan>,
    ) -> Result<(), Infallible> {
        Ok(())
    }
}

impl ManagementCacheEviction for NoManagementEffects {
    type Error = Infallible;

    fn delete_cache(&self, _key: &str) -> Result<(), Infallible> {
        Ok(())
    }
}
