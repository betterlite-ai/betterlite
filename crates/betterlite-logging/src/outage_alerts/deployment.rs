use super::OutageDetector;
use super::decision::{OutageAlertState, alert_type_for, append_status, outage_status};
use super::message::{OutageAlertType, OutageKey, OutageMessage, outage_alert_message};
use super::ports::{
    DeploymentWrite, OutageApiBase, OutageClock, OutageDeployment, OutageDeployments,
    OutageProviders, OutageSender, OutageStore,
};
use super::record::OutageRecord;

struct OutageView<'view> {
    model: &'view str,
    provider: &'view str,
    api_base: Option<&'view str>,
}

// One run borrows the detector for a single failure callback.
pub(super) struct Run<'run, Store, Deployments, Providers, ApiBase, Sender, Clock> {
    pub(super) detector:
        &'run OutageDetector<'run, Store, Deployments, Providers, ApiBase, Sender, Clock>,
}

impl<Store, Deployments, Providers, ApiBase, Sender, Clock>
    Run<'_, Store, Deployments, Providers, ApiBase, Sender, Clock>
where
    Store: OutageStore,
    Deployments: OutageDeployments,
    Providers: OutageProviders,
    ApiBase: OutageApiBase,
    Sender: OutageSender,
    Clock: OutageClock,
{
    // LiteLLM wraps this whole body in a bare except, so every failure below ends the call without raising.
    pub(super) async fn outage_alerts(&self, status_code: Option<i64>, deployment_id: &str) {
        let Ok(cached) = self.detector.store.get_outage_record(deployment_id).await else {
            return;
        };
        let Some(code) = outage_status(status_code) else {
            return;
        };
        let Some(registry) = self.detector.deployments else {
            return;
        };
        let Ok(Some(deployment)) = registry.deployment(deployment_id) else {
            return;
        };
        let (model, provider) = self.model_and_provider(&deployment);
        let Ok(api_base) = self.detector.api_base.api_base(&model, deployment_id) else {
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
        let Some(provider) = deployment.custom_llm_provider.as_deref() else {
            return match self.detector.providers.get_llm_provider(&deployment.model) {
                Ok(resolved) => (resolved.model, resolved.provider),
                Err(_resolution) => (deployment.model.clone(), String::new()),
            };
        };
        (deployment.model.clone(), provider.to_owned())
    }

    async fn store_first_deployment_record(&self, code: i64, deployment_id: &str) {
        let record = OutageRecord {
            model_id: deployment_id.to_owned(),
            alerts: vec![code],
            minor_alert_sent: false,
            major_alert_sent: false,
            last_updated_at: self.detector.clock.now_unix_seconds(),
        };
        let _stored = self
            .detector
            .store
            .set_outage_record(DeploymentWrite {
                key: deployment_id,
                record: &record,
                ttl_seconds: Some(self.detector.args.outage_alert_ttl),
            })
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
            self.detector.args.max_outage_alert_list_size,
        );
        record.last_updated_at = self.detector.clock.now_unix_seconds();
        let state = OutageAlertState {
            alerts: &record.alerts,
            minor_alert_sent: record.minor_alert_sent,
            major_alert_sent: record.major_alert_sent,
        };
        if let Some(alert_type) = alert_type_for(&state, self.detector.args, true) {
            if !self.send_deployment_alert(view, alert_type, record).await {
                return;
            }
            match alert_type {
                OutageAlertType::Minor => record.minor_alert_sent = true,
                OutageAlertType::Major => record.major_alert_sent = true,
            }
        }
        let _stored = self
            .detector
            .store
            .set_outage_record(DeploymentWrite {
                key: deployment_id,
                record,
                ttl_seconds: None,
            })
            .await;
    }

    async fn send_deployment_alert(
        &self,
        view: &OutageView<'_>,
        alert_type: OutageAlertType,
        record: &OutageRecord,
    ) -> bool {
        let message = OutageMessage {
            alert_type,
            key: OutageKey::Model,
            key_value: view.model,
            provider: view.provider,
            api_base: view.api_base,
            alerts: &record.alerts,
            now: self.detector.clock.now_unix_seconds(),
            last_updated_at: record.last_updated_at,
        };
        self.detector
            .sender
            .send_outage_alert(&outage_alert_message(&message), alert_type.level())
            .await
            .is_ok()
    }
}
