#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::database_failures::metrics::WallClock;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::ptu::rollup::alert::OperatorAlert;
use betterlite_db::ptu::rollup::scheduled::PTU_ROLLUP_LOCK_TTL_SECONDS;
use betterlite_db::queues::pod_lock::owner::PodId;
use betterlite_db::queues::pod_lock::{
    LockCache, LockCacheError, LockScript, NoPodLockEventSink, PodLockManager,
};
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::openai_base_config::Object as JsonMap;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_secret_managers::oidc::http_timeouts::OidcHttpTimeouts;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcTokens as _, OidcClock as _, OidcHttp as _, OidcRequest,
};
use chrono::{DateTime, FixedOffset, Utc};
use futures_util::future::BoxFuture;
use rstest::rstest;
use secrecy::SecretString;
use tokio_util::task::TaskTracker;
use uuid::Uuid;

use super::PTU_ROLLUP_SCHEDULE;
use super::attribution::{EnvironmentAttribution, NoOidc, PtuAttribution};
use super::snapshot::CatalogSnapshot;
use super::task::{PtuRollupInputs, ScheduledPtuRollup};
use crate::failure_hook::alerts::ProxyAlerts;
use crate::failure_hook::ports::{
    AlertLevel, AlertType, AlertingClient, BudgetAlertKind, CallInfo, Clock as AlertClock,
    FailureService, ServiceLogging, SlackAlert, SlackAlerting, UsageCache,
};

const FLAG: &str = "LITELLM_ENABLE_PTU_COST_ATTRIBUTION";

struct AlwaysAttribution(bool);

impl PtuAttribution for AlwaysAttribution {
    fn enabled(&self) -> bool {
        self.0
    }
}

struct FixedWallClock;

impl WallClock for FixedWallClock {
    fn now(&self) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-03-01T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the test has no alert sink")]
struct NoAlertError;

struct NoAlert;

impl OperatorAlert for NoAlert {
    type Error = NoAlertError;

    fn send<'message>(
        &'message self,
        _message: &'message str,
    ) -> BoxFuture<'message, Result<(), NoAlertError>> {
        Box::pin(async { Ok(()) })
    }
}

#[derive(Default)]
struct RecordingLease {
    owner: Mutex<Option<String>>,
    acquired: Mutex<Vec<String>>,
    deletes: Mutex<usize>,
}

impl RecordingLease {
    fn acquired(&self) -> Vec<String> {
        self.acquired.lock().unwrap().clone()
    }

    fn deletes(&self) -> usize {
        *self.deletes.lock().unwrap()
    }
}

impl LockCache for RecordingLease {
    fn get_owner(
        &self,
        _key: &str,
    ) -> impl Future<Output = Result<Option<Vec<u8>>, LockCacheError>> + Send {
        ready(Ok(self
            .owner
            .lock()
            .unwrap()
            .clone()
            .map(String::into_bytes)))
    }

    fn set_owner_if_absent(
        &self,
        _key: &str,
        owner: &str,
        _ttl_seconds: i64,
    ) -> impl Future<Output = Result<bool, LockCacheError>> + Send {
        let mut held = self.owner.lock().unwrap();
        if held.is_some() {
            return ready(Ok(false));
        }
        *held = Some(owner.to_owned());
        drop(held);
        self.acquired.lock().unwrap().push(owner.to_owned());
        ready(Ok(true))
    }

    fn delete(&self, _key: &str) -> impl Future<Output = Result<i64, LockCacheError>> + Send {
        let removed = self.owner.lock().unwrap().take();
        let mut deletes = self.deletes.lock().unwrap();
        *deletes = deletes.saturating_add(1);
        drop(deletes);
        ready(Ok(i64::from(removed.is_some())))
    }

    fn compare_and_delete_script(&self) -> Option<Arc<dyn LockScript>> {
        None
    }
}

fn manager(lease: &Arc<RecordingLease>) -> PodLockManager<RecordingLease> {
    PodLockManager::new(
        Some(Arc::clone(lease)),
        PTU_ROLLUP_LOCK_TTL_SECONDS,
        Arc::new(NoPodLockEventSink),
        PodId::try_from(Uuid::new_v4()).unwrap(),
    )
}

fn job(
    lease: &Arc<RecordingLease>,
    attribution: bool,
) -> Arc<ScheduledPtuRollup<RecordingLease, NoAlert>> {
    Arc::new(ScheduledPtuRollup {
        inputs: PtuRollupInputs {
            store: PostgresStore { pool: None },
            deployments: Arc::new(CatalogSnapshot::default()),
            lock: Arc::new(manager(lease)),
            alerts: None,
            attribution: Arc::new(AlwaysAttribution(attribution)),
            clock: Arc::new(FixedWallClock),
        },
    })
}

#[tokio::test]
async fn one_enabled_job_runs_each_tick_under_the_same_lock_owner() {
    let lease = Arc::new(RecordingLease::default());
    let job = job(&lease, true);
    let registration = job.registration().expect("attribution is on");
    assert_eq!(registration.schedule, PTU_ROLLUP_SCHEDULE);
    assert_eq!(registration.schedule.id, "ptu_flat_cost_rollup_job");
    assert_eq!(registration.schedule.crontab, "15 0 * * *");
    assert_eq!(registration.schedule.timezone, "UTC");
    assert!(registration.schedule.replace_existing);
    assert_eq!(registration.schedule.misfire_grace_seconds, None);
    let run = Arc::clone(&registration.run);
    run().await;
    run().await;
    let acquired = lease.acquired();
    assert_eq!(acquired.len(), 2);
    assert_eq!(acquired[0], acquired[1]);
    assert_eq!(lease.deletes(), 2);
}

#[tokio::test]
async fn a_disabled_flag_registers_no_job() {
    let lease = Arc::new(RecordingLease::default());
    let job = job(&lease, false);
    assert!(job.registration().is_none());
    assert!(lease.acquired().is_empty());
}

#[test]
fn the_snapshot_dumps_a_deployment_spec_as_a_router_entry() {
    let model_info: JsonObject = serde_json::from_str(
        r#"{"team_id":"team-1","ptu_count":2,"tags":[true,null],"extra":{"cost":"1.5"}}"#,
    )
    .unwrap();
    let spec = DeploymentSpec::new(
        "cfg-name".to_owned(),
        JsonObject::new(),
        model_info,
        JsonObject::new(),
        || "cfg-id".to_owned(),
    )
    .unwrap();
    let snapshot = CatalogSnapshot::from_specs(&[spec]);
    let entries = snapshot.entries;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].model_name, "cfg-name");
    let expected: JsonData = serde_json::from_str(
        r#"{"team_id":"team-1","ptu_count":2,"tags":[true,null],"extra":{"cost":"1.5"},"id":"cfg-id","db_model":false,"member_auto_router":false}"#,
    )
    .unwrap();
    assert_eq!(entries[0].model_info, expected);
}

#[rstest]
#[case::exact(Some("true"), true)]
#[case::uppercase(Some("TRUE"), true)]
#[case::padded_false(Some(" false "), false)]
#[case::one(Some("1"), false)]
#[case::empty(Some(""), false)]
#[case::absent(None, false)]
fn the_flag_reads_exactly_true_from_the_configured_environment(
    #[case] value: Option<&str>,
    #[case] expected: bool,
) {
    let attribution = EnvironmentAttribution {
        configured: BTreeMap::from([(
            FLAG.to_owned(),
            value.map(|text| SecretString::from(text.to_owned())),
        )]),
    };
    assert_eq!(attribution.enabled(), expected, "{value:?}");
}

#[test]
fn the_null_oidc_ports_answer_absent_and_read_the_system_clock() {
    let oidc = NoOidc;
    let before = Instant::now();
    assert!(oidc.now() >= before);
    assert!(oidc.unix_seconds() > 1_000_000_000_i64);
    assert!(oidc.token("scope").unwrap().is_none());
    let request = OidcRequest {
        url: "https://example.invalid/token".to_owned(),
        query: Vec::new(),
        headers: Vec::new(),
        timeouts: OidcHttpTimeouts::DEFAULT,
    };
    assert!(oidc.get(request).is_err());
}

#[derive(Default)]
struct NoUsage;

#[async_trait]
impl UsageCache for NoUsage {
    async fn set_local(&self, _key: &str, _value: &str, _ttl: Duration) {}
}

#[derive(Default)]
struct RecordingSlack {
    sent: Mutex<Vec<(AlertLevel, AlertType, String)>>,
}

#[async_trait]
impl SlackAlerting for RecordingSlack {
    fn alerting_configured(&self) -> bool {
        true
    }

    async fn failed_tracking_alert(&self, _error_message: &str, _failing_model: &str) {}

    async fn budget_alerts(&self, _kind: BudgetAlertKind, _user_info: &CallInfo) {}

    async fn send_alert(&self, alert: SlackAlert<'_>) {
        self.sent
            .lock()
            .unwrap()
            .push((alert.level, alert.alert_type, alert.message.to_owned()));
    }

    async fn response_taking_too_long(&self, _data: Option<JsonMap>) {}
}

struct NoLogging;

#[async_trait]
impl ServiceLogging for NoLogging {
    async fn async_service_failure_hook(
        &self,
        _service: FailureService,
        _duration: Duration,
        _error: &str,
        _call_type: &str,
    ) {
    }
}

struct FixedAlertClock;

impl AlertClock for FixedAlertClock {
    fn now(&self) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339("2026-03-01T00:15:00Z").unwrap()
    }
}

fn proxy_alerts(slack: Arc<RecordingSlack>) -> ProxyAlerts {
    ProxyAlerts {
        alerting: Some(vec![AlertingClient::Slack]),
        alert_types: Vec::new(),
        alerting_threshold: Duration::ZERO,
        proxy_base_url: None,
        usage_cache: Arc::new(NoUsage),
        slack,
        email: None,
        service_logging: Arc::new(NoLogging),
        sentry: None,
        redactor: None,
        clock: Arc::new(FixedAlertClock),
        tasks: TaskTracker::new(),
    }
}

#[tokio::test]
async fn the_alert_bridge_sends_high_failed_tracking_spend() {
    let slack = Arc::new(RecordingSlack::default());
    let alerts = proxy_alerts(Arc::clone(&slack));
    alerts
        .send("PTU flat-cost rollup for 2026-02-28: 1 of 2 team charges failed to write.")
        .await
        .unwrap();
    let sent = slack.sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, AlertLevel::High);
    assert_eq!(sent[0].1, AlertType::FailedTrackingSpend);
    assert!(sent[0].2.contains("1 of 2 team charges"), "{:?}", sent[0].2);
}
