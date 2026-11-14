use std::collections::BTreeSet;

use betterlite_models::deployment_regions::{RegionParams, get_model_region};

use super::OutageDetector;
use super::decision::{OutageAlertState, alert_type_for, append_status, outage_status};
use super::error::RegionOutageError;
use super::message::{OutageAlertType, OutageKey, OutageMessage, outage_alert_message};
use super::ports::{
    OutageClock, OutageDeployment, OutageDeployments, OutageProviders, OutageSender, OutageStore,
};
use super::record::RegionOutageRecord;

struct RegionTarget {
    cache_key: String,
    region_name: String,
    provider: String,
}

fn model_region(deployment: &OutageDeployment, provider: &str) -> Option<String> {
    let params = RegionParams {
        model: &deployment.model,
        custom_llm_provider: Some(provider),
        api_base: None,
        api_key: None,
        use_litellm_proxy: false,
        region_name: deployment.region_name.as_deref(),
        vertex_location: deployment.vertex_location.as_deref(),
        aws_region_name: deployment.aws_region_name.as_deref(),
        watsonx_region_name: deployment.watsonx_region_name.as_deref(),
    };
    get_model_region(provider, &params, None).map(str::to_owned)
}

impl<Store, Deployments, Providers, ApiBase, Sender, Clock>
    OutageDetector<'_, Store, Deployments, Providers, ApiBase, Sender, Clock>
where
    Store: OutageStore,
    Deployments: OutageDeployments,
    Providers: OutageProviders,
    Sender: OutageSender,
    Clock: OutageClock,
{
    // Unlike the deployment detector, failures here go back to the caller.
    pub async fn region_outage_alerts(
        &self,
        status_code: Option<i64>,
        deployment_id: &str,
    ) -> Result<
        (),
        RegionOutageError<Deployments::Error, Providers::Error, Store::Error, Sender::Error>,
    > {
        let Some(target) = self.region_target(deployment_id)? else {
            return Ok(());
        };
        let cached = self
            .store
            .get_region_outage_record(&target.cache_key)
            .await
            .map_err(RegionOutageError::Read)?;
        let Some(code) = outage_status(status_code) else {
            return Ok(());
        };
        let Some(mut record) = cached else {
            return self
                .store_first_region_record(&target.cache_key, code, deployment_id)
                .await;
        };
        self.update_region_record(&target, code, deployment_id, &mut record)
            .await
    }

    fn region_target(
        &self,
        deployment_id: &str,
    ) -> Result<
        Option<RegionTarget>,
        RegionOutageError<Deployments::Error, Providers::Error, Store::Error, Sender::Error>,
    > {
        let Some(registry) = self.deployments else {
            return Ok(None);
        };
        let found = registry
            .deployment(deployment_id)
            .map_err(RegionOutageError::Deployments)?;
        let Some(deployment) = found else {
            return Ok(None);
        };
        let provider = match deployment.custom_llm_provider.as_deref() {
            Some(provider) => provider.to_owned(),
            None => {
                self.providers
                    .get_llm_provider(&deployment.model)
                    .map_err(RegionOutageError::Providers)?
                    .provider
            }
        };
        let Some(region_name) = deployment
            .region_name
            .clone()
            .or_else(|| model_region(&deployment, &provider))
        else {
            return Ok(None);
        };
        let cache_key = format!("{provider}{region_name}");
        Ok(Some(RegionTarget {
            cache_key,
            region_name,
            provider,
        }))
    }

    async fn store_first_region_record(
        &self,
        cache_key: &str,
        code: i64,
        deployment_id: &str,
    ) -> Result<
        (),
        RegionOutageError<Deployments::Error, Providers::Error, Store::Error, Sender::Error>,
    > {
        let mut ids = BTreeSet::new();
        ids.insert(deployment_id.to_owned());
        let record = RegionOutageRecord {
            provider_region_id: cache_key.to_owned(),
            alerts: vec![code],
            minor_alert_sent: false,
            major_alert_sent: false,
            last_updated_at: self.clock.now_unix_seconds(),
            deployment_ids: ids,
        };
        self.store
            .set_region_outage_record(cache_key, &record, Some(self.args.region_outage_alert_ttl))
            .await
            .map_err(RegionOutageError::Write)
    }

    async fn update_region_record(
        &self,
        target: &RegionTarget,
        code: i64,
        deployment_id: &str,
        record: &mut RegionOutageRecord,
    ) -> Result<
        (),
        RegionOutageError<Deployments::Error, Providers::Error, Store::Error, Sender::Error>,
    > {
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
            let last_updated_at = record.last_updated_at;
            self.send_region_alert(target, alert_type, &record.alerts, last_updated_at)
                .await
                .map_err(RegionOutageError::Send)?;
            match alert_type {
                OutageAlertType::Minor => record.minor_alert_sent = true,
                OutageAlertType::Major => record.major_alert_sent = true,
            }
        }
        self.store
            .set_region_outage_record(&target.cache_key, record, None)
            .await
            .map_err(RegionOutageError::Write)?;
        Ok(())
    }

    async fn send_region_alert(
        &self,
        target: &RegionTarget,
        alert_type: OutageAlertType,
        alerts: &[i64],
        last_updated_at: f64,
    ) -> Result<(), Sender::Error> {
        let message = OutageMessage {
            alert_type,
            key: OutageKey::Region,
            key_value: &target.region_name,
            provider: &target.provider,
            api_base: None,
            alerts,
            now: self.clock.now_unix_seconds(),
            last_updated_at,
        };
        self.sender
            .send_outage_alert(&outage_alert_message(&message), alert_type.level())
            .await
    }
}
