use super::OutageDetector;
use super::decision::{OutageAlertState, alert_type_for, append_status, outage_status};
use super::message::{OutageAlertType, OutageKey, OutageMessage, outage_alert_message};
use super::ports::{
    OutageApiBase, OutageClock, OutageDeployment, OutageDeployments, OutageProviders, OutageSender,
    OutageStore,
};
use super::record::OutageRecord;

struct OutageView<'view> {
    model: &'view str,
    provider: &'view str,
    api_base: Option<&'view str>,
}

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
        let Some(registry) = self.deployments else {
            return;
        };
        let Ok(Some(deployment)) = registry.deployment(deployment_id) else {
            return;
        };
        let (model, provider) = self.model_and_provider(&deployment);
        let Ok(api_base) = self.api_base.api_base(&model, deployment_id) else {
            return;
        };
        let view = OutageView {
            model: &model,
            provider: &provider,
            api_base: api_base.as_deref(),
        };
        let Some(mut record) = cached else {
            self.store_first_deployment_record(code, deployment_id)
                .await;
            return;
        };
        self.update_deployment_record(&view, code, deployment_id, &mut record)
            .await;
    }

    fn model_and_provider(&self, deployment: &OutageDeployment) -> (String, String) {
        let mut model = deployment.model.clone();
        let provider = match deployment.custom_llm_provider.as_deref() {
            Some(provider) => provider.to_owned(),
            None => match self.providers.get_llm_provider(&deployment.model) {
                Ok(resolved) => {
                    model = resolved.model;
                    resolved.provider
                }
                Err(_resolution) => String::new(),
            },
        };
        (model, provider)
    }

    async fn store_first_deployment_record(&self, code: i64, deployment_id: &str) {
        let record = OutageRecord {
            model_id: deployment_id.to_owned(),
            alerts: vec![code],
            minor_alert_sent: false,
            major_alert_sent: false,
            last_updated_at: self.clock.now_unix_seconds(),
        };
        let _stored = self
            .store
            .set_outage_record(deployment_id, &record, Some(self.args.outage_alert_ttl))
            .await;
    }

    async fn update_deployment_record(
        &self,
        view: &OutageView<'_>,
        code: i64,
        deployment_id: &str,
        record: &mut OutageRecord,
    ) {
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
            let last_updated_at = record.last_updated_at;
            let sent = self
                .send_deployment_alert(view, alert_type, &record.alerts, last_updated_at)
                .await;
            if !sent {
                return;
            }
            match alert_type {
                OutageAlertType::Minor => record.minor_alert_sent = true,
                OutageAlertType::Major => record.major_alert_sent = true,
            }
        }
        let _stored = self
            .store
            .set_outage_record(deployment_id, record, None)
            .await;
    }

    async fn send_deployment_alert(
        &self,
        view: &OutageView<'_>,
        alert_type: OutageAlertType,
        alerts: &[i64],
        last_updated_at: f64,
    ) -> bool {
        let message = OutageMessage {
            alert_type,
            key: OutageKey::Model,
            key_value: view.model,
            provider: view.provider,
            api_base: view.api_base,
            alerts,
            now: self.clock.now_unix_seconds(),
            last_updated_at,
        };
        self.sender
            .send_outage_alert(&outage_alert_message(&message), alert_type.level())
            .await
            .is_ok()
    }
}
