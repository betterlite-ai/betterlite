use std::collections::BTreeMap;
use std::fs;
use std::str::FromStr as _;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_core::agents::json::AgentJson;
use betterlite_core::proxy::carried_budget_state::{
    KeyBudgetSnapshot, OrgBudgetSnapshot, TeamBudgetSnapshot, UserBudgetSnapshot,
};
use betterlite_logging::prometheus::budget_gauges::gauge::{
    BudgetGauges, EntityBudgetGauges, GaugeField,
};
use betterlite_prometheus::collector_factory::collector::MetricLike;
use betterlite_prometheus::no_op_metric::NoOpMetric;
use rust_decimal::Decimal;
use tempfile::NamedTempFile;
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum RecordedCall {
    Key {
        key: Option<String>,
        key_alias: Option<String>,
        response_cost: Decimal,
        max_budget: Option<Decimal>,
        spend: Option<Decimal>,
        carried: Option<KeyBudgetSnapshot>,
    },
    Team {
        team: Option<String>,
        team_alias: Option<String>,
        spend: Option<Decimal>,
        max_budget: Option<Decimal>,
        response_cost: Decimal,
        carried: Option<TeamBudgetSnapshot>,
    },
    User {
        user_id: Option<String>,
        spend: Option<Decimal>,
        max_budget: Option<Decimal>,
        response_cost: Decimal,
        carried: Option<UserBudgetSnapshot>,
        user_email: Option<String>,
    },
    Org {
        org_id: Option<String>,
        response_cost: Decimal,
        carried: Option<OrgBudgetSnapshot>,
        org_alias: Option<String>,
    },
    Customer {
        end_user_id: Option<String>,
        response_cost: Decimal,
    },
}

#[derive(Clone)]
pub(super) struct Recorder {
    inner: Arc<Inner>,
}

struct Inner {
    behaviour: BTreeMap<&'static str, Behaviour>,
    calls: Mutex<Vec<RecordedCall>>,
    finished: Mutex<Vec<&'static str>>,
}

impl Recorder {
    pub(super) fn new(behaviour: BTreeMap<&'static str, Behaviour>) -> Self {
        Self {
            inner: Arc::new(Inner {
                behaviour,
                calls: Mutex::new(Vec::new()),
                finished: Mutex::new(Vec::new()),
            }),
        }
    }

    pub(super) fn recording() -> Self {
        Self::new(BTreeMap::new())
    }

    pub(super) fn calls(&self) -> Vec<RecordedCall> {
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

    fn record(&self, call: RecordedCall) {
        self.inner
            .calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
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
                tokio::time::sleep(delay).await;
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
        self.record(RecordedCall::Key {
            key: lookup.key.map(str::to_owned),
            key_alias: lookup.key_alias.map(str::to_owned),
            response_cost: lookup.response_cost,
            max_budget: lookup.max_budget,
            spend: lookup.spend,
            carried: lookup.carried,
        });
        self.act("key").await
    }

    async fn team(&self, lookup: TeamBudgetLookup<'_>) -> Result<(), LookupError> {
        self.record(RecordedCall::Team {
            team: lookup.team.map(str::to_owned),
            team_alias: lookup.team_alias.map(str::to_owned),
            spend: lookup.spend,
            max_budget: lookup.max_budget,
            response_cost: lookup.response_cost,
            carried: lookup.carried,
        });
        self.act("team").await
    }

    async fn user(&self, lookup: UserBudgetLookup<'_>) -> Result<(), LookupError> {
        self.record(RecordedCall::User {
            user_id: lookup.user_id.map(str::to_owned),
            spend: lookup.spend,
            max_budget: lookup.max_budget,
            response_cost: lookup.response_cost,
            carried: lookup.carried,
            user_email: lookup.user_email.map(str::to_owned),
        });
        self.act("user").await
    }

    async fn org(&self, lookup: OrgBudgetLookup<'_>) -> Result<(), LookupError> {
        self.record(RecordedCall::Org {
            org_id: lookup.org_id.map(str::to_owned),
            response_cost: lookup.response_cost,
            carried: lookup.carried,
            org_alias: lookup.org_alias.map(str::to_owned),
        });
        self.act("org").await
    }

    async fn customer(&self, lookup: CustomerBudgetLookup<'_>) -> Result<(), LookupError> {
        self.record(RecordedCall::Customer {
            end_user_id: lookup.end_user_id.map(str::to_owned),
            response_cost: lookup.response_cost,
        });
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

fn entity_gauges(real: &[RealGauge]) -> [EntityBudgetGauges<MetricLike<EnabledGauge>>; 5] {
    [
        entity(
            real.contains(&RealGauge::Team),
            "litellm_remaining_team_budget_metric",
            "litellm_team_max_budget_metric",
            "litellm_team_budget_remaining_hours_metric",
        ),
        entity(
            real.contains(&RealGauge::Org),
            "litellm_remaining_org_budget_metric",
            "litellm_org_max_budget_metric",
            "litellm_org_budget_remaining_hours_metric",
        ),
        entity(
            real.contains(&RealGauge::Key),
            "litellm_remaining_api_key_budget_metric",
            "litellm_max_budget_metric",
            "litellm_budget_remaining_hours_metric",
        ),
        entity(
            real.contains(&RealGauge::User),
            "litellm_remaining_user_budget_metric",
            "litellm_user_max_budget_metric",
            "litellm_user_budget_remaining_hours_metric",
        ),
        EntityBudgetGauges {
            remaining: field(
                real.contains(&RealGauge::CustomerRemaining),
                "litellm_remaining_customer_budget_metric",
            ),
            max: field(
                real.contains(&RealGauge::CustomerMax),
                "litellm_customer_max_budget_metric",
            ),
            remaining_hours: field(
                real.contains(&RealGauge::CustomerHours),
                "litellm_customer_budget_remaining_hours_metric",
            ),
        },
    ]
}

fn entity(
    all_three: bool,
    remaining: &'static str,
    max: &'static str,
    hours: &'static str,
) -> EntityBudgetGauges<MetricLike<EnabledGauge>> {
    EntityBudgetGauges {
        remaining: field(all_three, remaining),
        max: field(all_three, max),
        remaining_hours: field(all_three, hours),
    }
}

fn field(enabled: bool, metric_name: &'static str) -> GaugeField<MetricLike<EnabledGauge>> {
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

pub(super) fn request(metadata: &BTreeMap<String, AgentJson>) -> RemainingBudgetRequest<'_> {
    RemainingBudgetRequest {
        team: Some("team-1"),
        team_alias: Some("Team"),
        key: Some("sk-hashed"),
        key_alias: Some("Key"),
        user_id: Some("user-1"),
        org_id: Some("org-1"),
        end_user_id: Some("end-1"),
        response_cost: Decimal::from_str("0.5").unwrap(),
        metadata,
    }
}

pub(super) fn metadata(entries: &[(&str, AgentJson)]) -> BTreeMap<String, AgentJson> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

pub(super) fn decimal(text: &str) -> Decimal {
    Decimal::from_str(text).unwrap()
}

pub(super) fn reset_at() -> chrono::DateTime<chrono::Utc> {
    "2026-11-20T02:00:00Z".parse().unwrap()
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
