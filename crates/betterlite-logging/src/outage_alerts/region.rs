use std::collections::BTreeSet;

use betterlite_models::deployment_regions::{RegionParams, get_model_region};

use super::OutageDetector;
use super::decision::{OutageAlertState, alert_type_for, append_status, outage_status};
use super::error::RegionOutageError;
use super::message::{OutageAlertType, OutageKey, OutageMessage, outage_alert_message};
use super::ports::{
    OutageApiBase, OutageClock, OutageDeployment, OutageDeployments, OutageProviders, OutageSender,
    OutageStore, RegionWrite,
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

fn region_provider<Providers: OutageProviders>(
    providers: &Providers,
    deployment: &OutageDeployment,
) -> Result<String, Providers::Error> {
    let Some(provider) = deployment.custom_llm_provider.as_deref() else {
        return Ok(providers.get_llm_provider(&deployment.model)?.provider);
    };
    Ok(provider.to_owned())
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
    fn region_target(
        &self,
        deployment: &OutageDeployment,
    ) -> Result<Option<RegionTarget>, Providers::Error> {
        let provider = region_provider(self.detector.providers, deployment)?;
        let Some(region_name) = deployment
            .region_name
            .clone()
            .or_else(|| model_region(deployment, &provider))
        else {
            return Ok(None);
        };
        Ok(Some(RegionTarget {
            cache_key: format!("{provider}{region_name}"),
            region_name,
            provider,
        }))
    }

    // Unlike the deployment detector, failures here go back to the caller.
    pub(super) async fn region_outage_alerts(
        &self,
        status_code: Option<i64>,
        deployment_id: &str,
    ) -> Result<
        (),
        RegionOutageError<Deployments::Error, Providers::Error, Store::Error, Sender::Error>,
    > {
        let Some(registry) = self.detector.deployments else {
            return Ok(());
        };
        let Some(deployment) = registry
            .deployment(deployment_id)
            .map_err(RegionOutageError::Deployments)?
        else {
            return Ok(());
        };
        let Some(target) = self
            .region_target(&deployment)
            .map_err(RegionOutageError::Providers)?
        else {
            return Ok(());
        };
        let cached = self
            .detector
            .store
            .get_region_outage_record(&target.cache_key)
            .await
            .map_err(RegionOutageError::Read)?;
        let Some(code) = outage_status(status_code) else {
            return Ok(());
        };
        let Some(mut record) = cached else {
            return self
                .store_first_region_record(&target, code, deployment_id)
                .await;
        };
        self.update_region_record(&target, code, deployment_id, &mut record)
            .await
    }

    async fn store_first_region_record(
        &self,
        target: &RegionTarget,
        code: i64,
        deployment_id: &str,
    ) -> Result<
        (),
        RegionOutageError<Deployments::Error, Providers::Error, Store::Error, Sender::Error>,
    > {
        let mut ids = BTreeSet::new();
        ids.insert(deployment_id.to_owned());
        let record = RegionOutageRecord {
            provider_region_id: target.cache_key.clone(),
            alerts: vec![code],
            minor_alert_sent: false,
            major_alert_sent: false,
            last_updated_at: self.detector.clock.now_unix_seconds(),
            deployment_ids: ids,
        };
        self.detector
            .store
            .set_region_outage_record(RegionWrite {
                key: &target.cache_key,
                record: &record,
                ttl_seconds: Some(self.detector.args.region_outage_alert_ttl),
            })
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
            self.detector.args.max_outage_alert_list_size,
        );
        record.deployment_ids.insert(deployment_id.to_owned());
        record.last_updated_at = self.detector.clock.now_unix_seconds();
        self.send_region_alert_if_due(target, record)
            .await
            .map_err(RegionOutageError::Send)?;
        self.detector
            .store
            .set_region_outage_record(RegionWrite {
                key: &target.cache_key,
                record,
                ttl_seconds: None,
            })
            .await
            .map_err(RegionOutageError::Write)?;
        Ok(())
    }

    async fn send_region_alert_if_due(
        &self,
        target: &RegionTarget,
        record: &mut RegionOutageRecord,
    ) -> Result<(), Sender::Error> {
        let state = OutageAlertState {
            alerts: &record.alerts,
            minor_alert_sent: record.minor_alert_sent,
            major_alert_sent: record.major_alert_sent,
        };
        let multiple_deployments = record.deployment_ids.len() > 1;
        let Some(alert_type) = alert_type_for(&state, self.detector.args, multiple_deployments)
        else {
            return Ok(());
        };
        self.send_region_alert(target, alert_type, record).await?;
        match alert_type {
            OutageAlertType::Minor => record.minor_alert_sent = true,
            OutageAlertType::Major => record.major_alert_sent = true,
        }
        Ok(())
    }

    async fn send_region_alert(
        &self,
        target: &RegionTarget,
        alert_type: OutageAlertType,
        record: &RegionOutageRecord,
    ) -> Result<(), Sender::Error> {
        let message = OutageMessage {
            alert_type,
            key: OutageKey::Region,
            key_value: &target.region_name,
            provider: &target.provider,
            api_base: None,
            alerts: &record.alerts,
            now: self.detector.clock.now_unix_seconds(),
            last_updated_at: record.last_updated_at,
        };
        self.detector
            .sender
            .send_outage_alert(&outage_alert_message(&message), alert_type.level())
            .await
    }
}
