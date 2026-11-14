use betterlite_config::sections::alert_settings::types::AlertType;

use super::OutageDetector;
use super::ports::{
    OutageApiBase, OutageClock, OutageDeployments, OutageProviders, OutageSender, OutageStore,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailureKind {
    ApiError { status_code: Option<i64> },
    Other,
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
    pub async fn on_failure(
        &self,
        alert_types: &[AlertType],
        failure: FailureKind,
        deployment_id: &str,
    ) {
        let FailureKind::ApiError { status_code } = failure else {
            return;
        };
        if alert_types.contains(&AlertType::OutageAlerts) {
            self.outage_alerts(status_code, deployment_id).await;
        }
        if alert_types.contains(&AlertType::RegionOutageAlerts) {
            let _dropped = self.region_outage_alerts(status_code, deployment_id).await;
        }
    }
}
