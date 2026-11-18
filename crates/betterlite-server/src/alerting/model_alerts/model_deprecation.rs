use std::error::Error as StdError;
use std::pin::pin;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::sections::alert_settings::types::AlertType;
use betterlite_logging::alert_dispatch::destination::AlertDestination;
use betterlite_logging::alert_dispatch::{Alert, AlertLevel};
use betterlite_logging::alert_settings::cache_keys::SlackAlertingCacheKeys;
use betterlite_models::decisions::Json;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use futures_util::future::{Either, select};
use indexmap::IndexMap;
use serde_json::Number;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;

use super::ModelAlertSender;
use crate::model_deprecation::alert::format_alert_message;
use crate::model_deprecation::catalog::DeprecationCatalog;
use crate::model_deprecation::clock::UtcClock;
use crate::model_deprecation::report::collect_model_deprecations;
use crate::model_deprecation::resolve::Deployment;
use crate::model_deprecation::types::{
    DEFAULT_CHECK_INTERVAL_SECONDS, DEFAULT_DEPRECATION_WARN_DAYS, DEPRECATION_IDLE_POLL_SECONDS,
    ModelDeprecationResponse,
};

pub const DEPRECATION_LOCK_ID: &str = "slack_model_deprecation_warning";
// B1690's acquire_lock takes its ttl as i64, where B843's day constant is a u64
const DEPRECATION_LOCK_TTL: i64 = 86_400;

#[async_trait]
pub trait DeprecationAlertCache: Send + Sync {
    async fn get(&self, key: &str) -> Option<Json>;

    async fn set(&self, key: &str, value: &Json, ttl_seconds: u64);
}

#[async_trait]
pub trait AlertPodLock: Send + Sync {
    async fn acquire_lock(
        &self,
        cronjob_id: &str,
        ttl: Option<i64>,
        allow_reentrant: bool,
    ) -> Option<bool>;
}

pub trait DeprecationRouterSource {
    fn deployments(&self) -> Option<Vec<DeploymentSpec>>;
}

pub trait ModelCatalogSource {
    fn deprecation_catalog(&self) -> &dyn DeprecationCatalog;
}

#[async_trait]
pub trait AlertSleeper: Send + Sync {
    // false when the check was cancelled while it waited
    async fn sleep(&self, seconds: u64) -> bool;
}

pub struct CancellationSleeper {
    pub cancellation: CancellationToken,
}

#[async_trait]
impl AlertSleeper for CancellationSleeper {
    async fn sleep(&self, seconds: u64) -> bool {
        let deadline = sleep(Duration::from_secs(seconds));
        match select(pin!(deadline), pin!(self.cancellation.cancelled())).await {
            Either::Left(((), _)) => true,
            Either::Right(((), _)) => false,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ModelDeprecationAlertError<E>
where
    E: StdError + 'static,
{
    #[error("the model deprecation alert could not be sent: {0}")]
    Alert(#[source] E),
    #[error("the deprecation alert stamp is not a JSON number")]
    Stamp,
}

pub struct ModelDeprecationAlerts<Cache, Sender, Clock, Sleeper, Router, Catalog> {
    pub alerting: Option<Vec<AlertDestination>>,
    pub alert_types: Vec<AlertType>,
    pub cache: Cache,
    pub sender: Sender,
    pub clock: Clock,
    pub sleeper: Sleeper,
    pub router_source: Router,
    pub catalog_source: Catalog,
}

impl<Cache, Sender, Clock, Sleeper, Router, Catalog>
    ModelDeprecationAlerts<Cache, Sender, Clock, Sleeper, Router, Catalog>
where
    Cache: DeprecationAlertCache,
    Sender: ModelAlertSender,
    Clock: UtcClock + Send + Sync,
    Sleeper: AlertSleeper,
    Router: DeprecationRouterSource + Send + Sync,
    Catalog: ModelCatalogSource + Send + Sync,
{
    #[must_use]
    pub fn deprecation_alerts_enabled(&self) -> bool {
        if self.alerting.is_none() {
            return false;
        }
        self.alert_types
            .contains(&AlertType::ModelDeprecationWarnings)
    }

    pub async fn send_model_deprecation_alert<Lock>(
        &self,
        deployments: Option<&[DeploymentSpec]>,
        pod_lock: Option<&Lock>,
    ) -> Result<bool, ModelDeprecationAlertError<Sender::Error>>
    where
        Lock: AlertPodLock,
    {
        if !self.deprecation_alerts_enabled() {
            return Ok(false);
        }
        let snapshot = self.deprecation_snapshot(deployments);
        let Some(message) = format_alert_message(&snapshot) else {
            return Ok(false);
        };
        if !self.claim_deprecation_window(pod_lock).await {
            return Ok(false);
        }
        let metadata = deprecation_alert_metadata(&snapshot);
        let kwargs = IndexMap::new();
        let alert = Alert {
            message: &message,
            level: deprecation_alert_level(&snapshot),
            alert_type: AlertType::ModelDeprecationWarnings,
            alerting_metadata: &metadata,
            user_info: None,
            request_model: None,
            api_base: None,
            kwargs: &kwargs,
        };
        self.sender
            .send_alert(alert)
            .await
            .map_err(ModelDeprecationAlertError::Alert)?;
        let stamp = self.deprecation_stamp()?;
        let key = SlackAlertingCacheKeys::ModelDeprecationAlertSent.to_string();
        self.cache
            .set(&key, &stamp, DEFAULT_CHECK_INTERVAL_SECONDS)
            .await;
        Ok(true)
    }

    pub async fn run_deprecation_alert_pass<Lock>(
        &self,
        deployments: Option<&[DeploymentSpec]>,
        pod_lock: Option<&Lock>,
    ) -> Result<bool, ModelDeprecationAlertError<Sender::Error>>
    where
        Lock: AlertPodLock,
    {
        if deployments.is_none() || !self.deprecation_alerts_enabled() {
            return Ok(false);
        }
        let key = SlackAlertingCacheKeys::ModelDeprecationAlertSent.to_string();
        if self.cache.get(&key).await.is_some() {
            return Ok(false);
        }
        self.send_model_deprecation_alert(deployments, pod_lock)
            .await
    }

    pub async fn run_deprecation_check_loop<Lock>(&self, pod_lock: Option<&Lock>)
    where
        Lock: AlertPodLock,
    {
        loop {
            let deployments = self.router_source.deployments();
            let outcome = self
                .run_deprecation_alert_pass(deployments.as_deref(), pod_lock)
                .await;
            let delay = match outcome {
                Ok(_) => DEPRECATION_IDLE_POLL_SECONDS,
                Err(error) => {
                    tracing::error!("Error in model deprecation alert loop: {error}");
                    DEFAULT_CHECK_INTERVAL_SECONDS
                }
            };
            if !self.sleeper.sleep(delay).await {
                return;
            }
        }
    }

    fn deprecation_snapshot(
        &self,
        deployments: Option<&[DeploymentSpec]>,
    ) -> ModelDeprecationResponse {
        let resolved: Vec<Deployment<'_>> = deployments
            .unwrap_or_default()
            .iter()
            .map(Deployment::from_spec)
            .collect();
        collect_model_deprecations(
            &resolved,
            self.catalog_source.deprecation_catalog(),
            &self.clock,
            DEFAULT_DEPRECATION_WARN_DAYS,
            None,
        )
    }

    async fn claim_deprecation_window<Lock>(&self, pod_lock: Option<&Lock>) -> bool
    where
        Lock: AlertPodLock,
    {
        let Some(lock) = pod_lock else {
            return true;
        };
        lock.acquire_lock(DEPRECATION_LOCK_ID, Some(DEPRECATION_LOCK_TTL), false)
            .await
            != Some(false)
    }

    fn deprecation_stamp(&self) -> Result<Json, ModelDeprecationAlertError<Sender::Error>> {
        let seconds = self.clock.now().timestamp();
        let value =
            num_traits::cast::<i64, f64>(seconds).ok_or(ModelDeprecationAlertError::Stamp)?;
        let number = Number::from_f64(value).ok_or(ModelDeprecationAlertError::Stamp)?;
        Ok(Json::Number(number))
    }
}

#[must_use]
pub const fn deprecation_alert_level(snapshot: &ModelDeprecationResponse) -> AlertLevel {
    if snapshot.deprecated.is_empty() {
        AlertLevel::Medium
    } else {
        AlertLevel::High
    }
}

#[must_use]
pub fn deprecation_alert_metadata(snapshot: &ModelDeprecationResponse) -> IndexMap<String, Json> {
    let mut metadata = IndexMap::new();
    metadata.insert(
        "deprecated_count".to_owned(),
        Json::Number(Number::from(snapshot.deprecated.len())),
    );
    metadata.insert(
        "imminent_count".to_owned(),
        Json::Number(Number::from(snapshot.imminent.len())),
    );
    metadata.insert(
        "upcoming_count".to_owned(),
        Json::Number(Number::from(snapshot.upcoming.len())),
    );
    metadata
}

mod tests;
