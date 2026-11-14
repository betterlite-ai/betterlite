use betterlite_config::sections::alert_settings::types::{AlertType, SlackAlertingArgs};

pub mod decision;
pub mod error;
pub mod message;
pub mod ports;
pub mod record;

mod deployment;
pub mod entry;
mod region;
mod tests;

use self::entry::FailureKind;
use self::error::RegionOutageError;
use self::ports::{
    OutageApiBase, OutageClock, OutageDeployments, OutageProviders, OutageSender, OutageStore,
};

pub struct OutageDetector<'ports, Store, Deployments, Providers, ApiBase, Sender, Clock> {
    pub store: &'ports Store,
    // None stands for LiteLLM's llm_router being None, where both detectors return before any lookup
    pub deployments: Option<&'ports Deployments>,
    pub providers: &'ports Providers,
    pub api_base: &'ports ApiBase,
    pub sender: &'ports Sender,
    pub clock: &'ports Clock,
    pub args: &'ports SlackAlertingArgs,
}

// One impl block: clippy rejects a type's inherent methods spread over several blocks.
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
    pub async fn outage_alerts(&self, status_code: Option<i64>, deployment_id: &str) {
        deployment::Run { detector: self }
            .outage_alerts(status_code, deployment_id)
            .await;
    }

    pub async fn region_outage_alerts(
        &self,
        status_code: Option<i64>,
        deployment_id: &str,
    ) -> Result<
        (),
        RegionOutageError<Deployments::Error, Providers::Error, Store::Error, Sender::Error>,
    > {
        region::Run { detector: self }
            .region_outage_alerts(status_code, deployment_id)
            .await
    }

    pub async fn on_failure(
        &self,
        alert_types: &[AlertType],
        failure: FailureKind,
        deployment_id: &str,
    ) {
        entry::on_failure(self, alert_types, failure, deployment_id).await;
    }
}
