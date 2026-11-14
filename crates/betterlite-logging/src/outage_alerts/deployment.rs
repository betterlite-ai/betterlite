use super::OutageDetector;
use super::decision::{OutageAlertState, alert_type_for, append_status, outage_status};
use super::message::{OutageAlertType, OutageKey, OutageMessage, outage_alert_message};
use super::ports::{
    OutageApiBase, OutageClock, OutageDeployments, OutageProviders, OutageSender, OutageStore,
};
use super::record::OutageRecord;

impl<Store, Deployments, Providers, ApiBase, Sender, Clock>
    OutageDetector<'_, Store, Deployments, Providers, ApiBase, Sender, Clock>
where
    Store: OutageStore,
    Deployments: OutageDeployments,
    Providers: OutageProviders,
    ApiBase: OutageApiBase,
    Sender: OutageSender,
    Clock: OutageClock,
{
    // LiteLLM wraps this whole body in a bare except, so every failure below ends the call without raising.
    pub async fn outage_alerts(&self, status_code: Option<i64>, deployment_id: &str) {
        let Ok(cached) = self.store.get_outage_record(deployment_id).await else {
            return;
        };
        let Some(code) = outage_status(status_code) else {
            return;
        };
        let Some(deployments) = self.deployments else {
            return;
        };
        let Ok(Some(deployment)) = deployments.deployment(deployment_id) else {
            return;
        };
        let mut model = deployment.model;
        let provider = match deployment.custom_llm_provider {
            Some(provider) => provider,
            None => match self.providers.get_llm_provider(&model) {
                Ok(resolved) => {
                    model = resolved.model;
                    resolved.provider
                }
                Err(_resolution) => String::new(),
            },
        };
        let Ok(api_base) = self.api_base.api_base(&model, deployment_id) else {
            return;
        };
        let Some(mut record) = cached else {
            let created = OutageRecord {
                model_id: deployment_id.to_owned(),
                alerts: vec![code],
                minor_alert_sent: false,
                major_alert_sent: false,
                last_updated_at: self.clock.now_unix_seconds(),
            };
            let _stored = self
                .store
                .set_outage_record(deployment_id, &created, Some(self.args.outage_alert_ttl))
                .await;
            return;
        };
        append_status(
            &mut record.alerts,
            code,
            self.args.max_outage_alert_list_size,
        );
        record.last_updated_at = self.clock.now_unix_seconds();
        let state = OutageAlertState {
            alerts: &record.alerts,
            minor_alert_sent: record.minor_alert_sent,
            major_alert_sent: record.major_alert_sent,
        };
        if let Some(alert_type) = alert_type_for(&state, self.args, true) {
            let message = OutageMessage {
                alert_type,
                key: OutageKey::Model,
                key_value: &model,
                provider: &provider,
                api_base: api_base.as_deref(),
                alerts: &record.alerts,
                now: self.clock.now_unix_seconds(),
                last_updated_at: record.last_updated_at,
            };
            let Ok(()) = self
                .sender
                .send_outage_alert(&outage_alert_message(&message), alert_type.level())
                .await
            else {
                return;
            };
            match alert_type {
                OutageAlertType::Minor => record.minor_alert_sent = true,
                OutageAlertType::Major => record.major_alert_sent = true,
            }
        }
        let _stored = self
            .store
            .set_outage_record(deployment_id, &record, None)
            .await;
    }
}
