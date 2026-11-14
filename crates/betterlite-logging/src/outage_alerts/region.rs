use std::collections::BTreeSet;

use betterlite_models::deployment_regions::{RegionParams, get_model_region};

use super::OutageDetector;
use super::decision::{OutageAlertState, alert_type_for, append_status, outage_status};
use super::error::RegionOutageError;
use super::message::{OutageAlertType, OutageKey, OutageMessage, outage_alert_message};
use super::ports::{OutageClock, OutageDeployments, OutageProviders, OutageSender, OutageStore};
use super::record::RegionOutageRecord;

impl<Store, Deployments, Providers, ApiBase, Sender, Clock>
    OutageDetector<'_, Store, Deployments, Providers, ApiBase, Sender, Clock>
where
    Store: OutageStore,
    Deployments: OutageDeployments,
    Providers: OutageProviders,
    Sender: OutageSender,
    Clock: OutageClock,
{
    // Unlike the per-deployment detector this one has no catch-all, so every failure goes back to the caller.
    pub async fn region_outage_alerts(
        &self,
        status_code: Option<i64>,
        deployment_id: &str,
    ) -> Result<
        (),
        RegionOutageError<Deployments::Error, Providers::Error, Store::Error, Sender::Error>,
    > {
        let Some(deployments) = self.deployments else {
            return Ok(());
        };
        let found = deployments
            .deployment(deployment_id)
            .map_err(RegionOutageError::Deployments)?;
        let Some(deployment) = found else {
            return Ok(());
        };
        let provider = match deployment.custom_llm_provider {
            Some(provider) => provider,
            None => {
                self.providers
                    .get_llm_provider(&deployment.model)
                    .map_err(RegionOutageError::Providers)?
                    .provider
            }
        };
        let params = RegionParams {
            model: &deployment.model,
            custom_llm_provider: Some(&provider),
            api_base: None,
            api_key: None,
            use_litellm_proxy: false,
            region_name: deployment.region_name.as_deref(),
            vertex_location: deployment.vertex_location.as_deref(),
            aws_region_name: deployment.aws_region_name.as_deref(),
            watsonx_region_name: deployment.watsonx_region_name.as_deref(),
        };
        let Some(region_name) = deployment
            .region_name
            .as_deref()
            .or_else(|| get_model_region(&provider, &params, None))
        else {
            return Ok(());
        };
        let cache_key = format!("{provider}{region_name}");
        let cached = self
            .store
            .get_region_outage_record(&cache_key)
            .await
            .map_err(RegionOutageError::Read)?;
        let Some(code) = outage_status(status_code) else {
            return Ok(());
        };
        let Some(mut record) = cached else {
            let mut deployment_ids = BTreeSet::new();
            deployment_ids.insert(deployment_id.to_owned());
            let created = RegionOutageRecord {
                provider_region_id: cache_key.clone(),
                alerts: vec![code],
                minor_alert_sent: false,
                major_alert_sent: false,
                last_updated_at: self.clock.now_unix_seconds(),
                deployment_ids,
            };
            self.store
                .set_region_outage_record(
                    &cache_key,
                    &created,
                    Some(self.args.region_outage_alert_ttl),
                )
                .await
                .map_err(RegionOutageError::Write)?;
            return Ok(());
        };
        append_status(
            &mut record.alerts,
            code,
            self.args.max_outage_alert_list_size,
        );
        record.deployment_ids.insert(deployment_id.to_owned());
        record.last_updated_at = self.clock.now_unix_seconds();
        let state = OutageAlertState {
            alerts: &record.alerts,
            minor_alert_sent: record.minor_alert_sent,
            major_alert_sent: record.major_alert_sent,
        };
        let multiple_deployments = record.deployment_ids.len() > 1;
        if let Some(alert_type) = alert_type_for(&state, self.args, multiple_deployments) {
            let message = OutageMessage {
                alert_type,
                key: OutageKey::Region,
                key_value: region_name,
                provider: &provider,
                api_base: None,
                alerts: &record.alerts,
                now: self.clock.now_unix_seconds(),
                last_updated_at: record.last_updated_at,
            };
            self.sender
                .send_outage_alert(&outage_alert_message(&message), alert_type.level())
                .await
                .map_err(RegionOutageError::Send)?;
            match alert_type {
                OutageAlertType::Minor => record.minor_alert_sent = true,
                OutageAlertType::Major => record.major_alert_sent = true,
            }
        }
        self.store
            .set_region_outage_record(&cache_key, &record, None)
            .await
            .map_err(RegionOutageError::Write)?;
        Ok(())
    }
}
