use std::collections::BTreeMap;
use std::fs;
use std::str::FromStr as _;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_core::agents::json::AgentJson;
use betterlite_logging::prometheus::budget_gauges::gauge::{
    BudgetGauges, EntityBudgetGauges, GaugeField,
};
use betterlite_prometheus::collector_factory::collector::MetricLike;
use betterlite_prometheus::no_op_metric::NoOpMetric;
use rust_decimal::Decimal;
use serde::Serialize;
use serde_json::to_value;
use tempfile::NamedTempFile;
use tokio::time::sleep;
use tracing::subscriber::{DefaultGuard, set_default};

use crate::prometheus_remaining_budget::fan_out::RemainingBudgetRequest;
use crate::prometheus_remaining_budget::lookups::{
    CustomerBudgetLookup, KeyBudgetLookup, LookupError, OrgBudgetLookup, RequestBudgetLookups,
    TeamBudgetLookup, UserBudgetLookup,
};

// any type the gauge holder takes; the fan-out only ever asks whether a collector is the no-op
pub(super) struct EnabledGauge;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum Behaviour {
    #[default]
    Fast,
    Slow(Duration),
    Failing,
}

#[derive(Clone)]
pub(super) struct Recorder {
    inner: Arc<Inner>,
}

struct Inner {
    behaviour: BTreeMap<&'static str, Behaviour>,
    calls: Mutex<BTreeMap<&'static str, AgentJson>>,
    finished: Mutex<Vec<&'static str>>,
}

impl Recorder {
    pub(super) fn new(behaviour: BTreeMap<&'static str, Behaviour>) -> Self {
        Self {
            inner: Arc::new(Inner {
                behaviour,
                calls: Mutex::new(BTreeMap::new()),
                finished: Mutex::new(Vec::new()),
            }),
        }
    }

    pub(super) fn recording() -> Self {
        Self::new(BTreeMap::new())
    }

    pub(super) fn calls(&self) -> BTreeMap<&'static str, AgentJson> {
        self.inner
            .calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn finished(&self) -> Vec<&'static str> {
        self.inner
            .finished
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record<T>(&self, entity: &'static str, lookup: &T)
    where
        T: Serialize,
    {
        let recorded = to_value(lookup).unwrap();
        self.inner
            .calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(entity, recorded);
    }

    fn mark_finished(&self, entity: &'static str) {
        self.inner
            .finished
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(entity);
    }

    async fn act(&self, entity: &'static str) -> Result<(), LookupError> {
        match self
            .inner
            .behaviour
            .get(entity)
            .copied()
            .unwrap_or_default()
        {
            Behaviour::Fast => {
                self.mark_finished(entity);
                Ok(())
            }
            Behaviour::Slow(delay) => {
                sleep(delay).await;
                self.mark_finished(entity);
                Ok(())
            }
            Behaviour::Failing => Err(LookupError {
                message: format!("{entity} lookup failed"),
            }),
        }
    }
}

#[async_trait]
impl RequestBudgetLookups for Recorder {
    async fn key(&self, lookup: KeyBudgetLookup<'_>) -> Result<(), LookupError> {
        self.record("key", &lookup);
        self.act("key").await
    }

    async fn team(&self, lookup: TeamBudgetLookup<'_>) -> Result<(), LookupError> {
        self.record("team", &lookup);
        self.act("team").await
    }

    async fn user(&self, lookup: UserBudgetLookup<'_>) -> Result<(), LookupError> {
        self.record("user", &lookup);
        self.act("user").await
    }

    async fn org(&self, lookup: OrgBudgetLookup<'_>) -> Result<(), LookupError> {
        self.record("org", &lookup);
        self.act("org").await
    }

    async fn customer(&self, lookup: CustomerBudgetLookup<'_>) -> Result<(), LookupError> {
        self.record("customer", &lookup);
        self.act("customer").await
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RealGauge {
    Team,
    Key,
    User,
    Org,
    CustomerRemaining,
    CustomerMax,
    CustomerHours,
}

pub(super) fn collectors_with(real: &[RealGauge]) -> BudgetGauges<MetricLike<EnabledGauge>> {
    let [team, org, key, user, customer] = entity_gauges(real);
    BudgetGauges {
        team,
        org,
        key,
        user,
        customer,
        team_members: field(false, "litellm_team_members"),
        provider_remaining: field(false, "litellm_provider_remaining"),
    }
}

// the gate reads the four remaining gauges only, so every max and hours gauge real must still skip
pub(super) fn collectors_without_remaining_gauges() -> BudgetGauges<MetricLike<EnabledGauge>> {
    let mut collectors = collectors_with(&[]);
    collectors.team.max = field(true, "litellm_team_max_budget_metric");
    collectors.team.remaining_hours = field(true, "litellm_team_budget_remaining_hours_metric");
    collectors.key.max = field(true, "litellm_max_budget_metric");
    collectors.key.remaining_hours = field(true, "litellm_budget_remaining_hours_metric");
    collectors.user.max = field(true, "litellm_user_max_budget_metric");
    collectors.user.remaining_hours = field(true, "litellm_user_budget_remaining_hours_metric");
    collectors.org.max = field(true, "litellm_org_max_budget_metric");
    collectors.org.remaining_hours = field(true, "litellm_org_budget_remaining_hours_metric");
    collectors
}

fn entity_gauges(real: &[RealGauge]) -> [EntityBudgetGauges<MetricLike<EnabledGauge>>; 5] {
    let [team, org, key, user, customer] = gauge_names();
    let [customer_remaining, customer_max, customer_hours] = customer;
    [
        entity(real.contains(&RealGauge::Team), team),
        entity(real.contains(&RealGauge::Org), org),
        entity(real.contains(&RealGauge::Key), key),
        entity(real.contains(&RealGauge::User), user),
        EntityBudgetGauges {
            remaining: field(
                real.contains(&RealGauge::CustomerRemaining),
                customer_remaining,
            ),
            max: field(real.contains(&RealGauge::CustomerMax), customer_max),
            remaining_hours: field(real.contains(&RealGauge::CustomerHours), customer_hours),
        },
    ]
}

const fn gauge_names() -> [[&'static str; 3]; 5] {
    [
        [
            "litellm_remaining_team_budget_metric",
            "litellm_team_max_budget_metric",
            "litellm_team_budget_remaining_hours_metric",
        ],
        [
            "litellm_remaining_org_budget_metric",
            "litellm_org_max_budget_metric",
            "litellm_org_budget_remaining_hours_metric",
        ],
        [
            "litellm_remaining_api_key_budget_metric",
            "litellm_max_budget_metric",
            "litellm_budget_remaining_hours_metric",
        ],
        [
            "litellm_remaining_user_budget_metric",
            "litellm_user_max_budget_metric",
            "litellm_user_budget_remaining_hours_metric",
        ],
        [
            "litellm_remaining_customer_budget_metric",
            "litellm_customer_max_budget_metric",
            "litellm_customer_budget_remaining_hours_metric",
        ],
    ]
}

const fn entity(
    all_three: bool,
    names: [&'static str; 3],
) -> EntityBudgetGauges<MetricLike<EnabledGauge>> {
    let [remaining, max, hours] = names;
    EntityBudgetGauges {
        remaining: field(all_three, remaining),
        max: field(all_three, max),
        remaining_hours: field(all_three, hours),
    }
}

const fn field(enabled: bool, metric_name: &'static str) -> GaugeField<MetricLike<EnabledGauge>> {
    let collector = if enabled {
        MetricLike::Real(EnabledGauge)
    } else {
        MetricLike::NoOp(NoOpMetric)
    };
    GaugeField {
        metric_name,
        supported_labels: Vec::new(),
        collector,
    }
}

pub(super) fn request(entries: &BTreeMap<String, AgentJson>) -> RemainingBudgetRequest<'_> {
    RemainingBudgetRequest {
        team: Some("team-1"),
        team_alias: Some("Team"),
        key: Some("sk-hashed"),
        key_alias: Some("Key"),
        user_id: Some("user-1"),
        org_id: Some("org-1"),
        end_user_id: Some("end-1"),
        response_cost: Decimal::from_str("0.5").unwrap(),
        metadata: entries,
    }
}

// every metadata key the fan-out reads, each set to a value of its declared type
pub(super) fn full_metadata() -> BTreeMap<String, AgentJson> {
    metadata(&[
        ("user_api_key_team_spend", AgentJson::from(3.0_f64)),
        ("user_api_key_team_max_budget", AgentJson::from(10.0_f64)),
        ("user_api_key_spend", AgentJson::from(1.5_f64)),
        ("user_api_key_max_budget", AgentJson::from(5.0_f64)),
        ("user_api_key_user_spend", AgentJson::from(0.25_f64)),
        ("user_api_key_user_max_budget", AgentJson::from(2.0_f64)),
        ("user_api_key_user_email", AgentJson::from("u@example.com")),
        ("user_api_key_org_alias", AgentJson::from("Org")),
        (
            "user_api_key_budget_reset_at",
            AgentJson::from("2026-11-20T02:00:00Z"),
        ),
        (
            "user_api_key_team_budget_reset_at",
            AgentJson::from("2026-11-20T02:00:00Z"),
        ),
        (
            "user_api_key_team_table_max_budget",
            AgentJson::from(10.0_f64),
        ),
        (
            "user_api_key_user_budget_reset_at",
            AgentJson::from("2026-11-20T02:00:00Z"),
        ),
        (
            "user_api_key_user_table_max_budget",
            AgentJson::from(2.0_f64),
        ),
        ("user_api_key_user_alias", AgentJson::from("Alias")),
        ("user_api_key_org_spend", AgentJson::from(0.5_f64)),
        ("user_api_key_org_max_budget", AgentJson::from(7.0_f64)),
    ])
}

pub(super) fn metadata(entries: &[(&str, AgentJson)]) -> BTreeMap<String, AgentJson> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

pub(super) fn capture_log() -> (DefaultGuard, NamedTempFile) {
    let log = NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    (set_default(subscriber), log)
}

pub(super) fn logged_text(log: &NamedTempFile) -> String {
    fs::read_to_string(log.path()).unwrap()
}
