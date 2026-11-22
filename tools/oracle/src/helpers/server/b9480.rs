use std::collections::BTreeMap;
use std::fmt::Display;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::loader::ConfigLoader;
use betterlite_core::agents::json::AgentJson;
use betterlite_logging::prometheus::budget_gauges::gauge::{
    BudgetGauges, EntityBudgetGauges, GaugeField,
};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_prometheus::collector_factory::collector::MetricLike;
use betterlite_prometheus::no_op_metric::NoOpMetric;
use betterlite_server::prometheus_remaining_budget::fan_out::{
    RemainingBudgetFanOut, RemainingBudgetRequest,
};
use betterlite_server::prometheus_remaining_budget::lookups::{
    CustomerBudgetLookup, KeyBudgetLookup, LookupError, OrgBudgetLookup, RequestBudgetLookups,
    TeamBudgetLookup, UserBudgetLookup,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json, to_value};
use tokio::runtime::Builder;

const TIMEOUT_VARIABLE: &str = "PROMETHEUS_BUDGET_METRICS_PER_REQUEST_TIMEOUT";
const HELPER_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) struct Op;

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum GaugeGroup {
    Team,
    Key,
    User,
    Org,
    Customer,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FanOutCall {
    team: Option<String>,
    team_alias: Option<String>,
    key: Option<String>,
    key_alias: Option<String>,
    user_id: Option<String>,
    org_id: Option<String>,
    end_user_id: Option<String>,
    response_cost: Decimal,
    metadata: BTreeMap<String, AgentJson>,
    noop_groups: Vec<GaugeGroup>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum Input {
    Timeout { values: Vec<Option<String>> },
    FanOut(FanOutCall),
}

// the fan-out only reads the collector variant of a gauge, so a unit stand-in stands for every real one
struct StandIn;

type CallLog = Arc<Mutex<BTreeMap<&'static str, Value>>>;

#[derive(Clone, Default)]
struct Recorder {
    calls: CallLog,
}

fn failure<E>(error: &E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "HelperFailure".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn configured_timeout(value: Option<&String>) -> Result<f64, Box<HelperError>> {
    let mut lookup = BTreeMap::new();
    if let Some(text) = value {
        lookup.insert(TIMEOUT_VARIABLE.to_owned(), text.clone());
    }
    let loaded = ConfigLoader { lookup }
        .load("model_list: []\n")
        .map_err(|error| failure(&error))?;
    Ok(loaded
        .sections
        .prometheus_request_budget
        .per_request_timeout
        .as_secs_f64())
}

fn timeout(values: &[Option<String>]) -> HelperResult {
    let mut resolved = Vec::with_capacity(values.len());
    for value in values {
        resolved.push(configured_timeout(value.as_ref())?);
    }
    Ok(json!(resolved))
}

const fn field(enabled: bool, metric_name: &'static str) -> GaugeField<MetricLike<StandIn>> {
    let collector = if enabled {
        MetricLike::Real(StandIn)
    } else {
        MetricLike::NoOp(NoOpMetric)
    };
    GaugeField {
        metric_name,
        supported_labels: Vec::new(),
        collector,
    }
}

const fn entity(group: GaugeGroup, enabled: bool) -> EntityBudgetGauges<MetricLike<StandIn>> {
    let [remaining, max, hours] = gauge_names(group);
    EntityBudgetGauges {
        remaining: field(enabled, remaining),
        max: field(enabled, max),
        remaining_hours: field(enabled, hours),
    }
}

const fn gauge_names(group: GaugeGroup) -> [&'static str; 3] {
    match group {
        GaugeGroup::Team => [
            "litellm_remaining_team_budget_metric",
            "litellm_team_max_budget_metric",
            "litellm_team_budget_remaining_hours_metric",
        ],
        GaugeGroup::Key => [
            "litellm_remaining_api_key_budget_metric",
            "litellm_max_budget_metric",
            "litellm_budget_remaining_hours_metric",
        ],
        GaugeGroup::User => [
            "litellm_remaining_user_budget_metric",
            "litellm_user_max_budget_metric",
            "litellm_user_budget_remaining_hours_metric",
        ],
        GaugeGroup::Org => [
            "litellm_remaining_org_budget_metric",
            "litellm_org_max_budget_metric",
            "litellm_org_budget_remaining_hours_metric",
        ],
        GaugeGroup::Customer => [
            "litellm_remaining_customer_budget_metric",
            "litellm_customer_max_budget_metric",
            "litellm_customer_budget_remaining_hours_metric",
        ],
    }
}

fn budget_gauges(noop_groups: &[GaugeGroup]) -> BudgetGauges<MetricLike<StandIn>> {
    BudgetGauges {
        team: entity(GaugeGroup::Team, !noop_groups.contains(&GaugeGroup::Team)),
        org: entity(GaugeGroup::Org, !noop_groups.contains(&GaugeGroup::Org)),
        key: entity(GaugeGroup::Key, !noop_groups.contains(&GaugeGroup::Key)),
        user: entity(GaugeGroup::User, !noop_groups.contains(&GaugeGroup::User)),
        customer: entity(
            GaugeGroup::Customer,
            !noop_groups.contains(&GaugeGroup::Customer),
        ),
        team_members: field(false, "litellm_team_members"),
        provider_remaining: field(false, "litellm_provider_remaining"),
    }
}

impl Recorder {
    fn record<T: Serialize>(&self, entity: &'static str, lookup: &T) -> Result<(), LookupError> {
        let recorded = to_value(lookup).map_err(|error| LookupError {
            message: error.to_string(),
        })?;
        let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls.insert(entity, recorded);
        Ok(())
    }
}

#[async_trait]
impl RequestBudgetLookups for Recorder {
    async fn key(&self, lookup: KeyBudgetLookup<'_>) -> Result<(), LookupError> {
        self.record("key", &lookup)
    }

    async fn team(&self, lookup: TeamBudgetLookup<'_>) -> Result<(), LookupError> {
        self.record("team", &lookup)
    }

    async fn user(&self, lookup: UserBudgetLookup<'_>) -> Result<(), LookupError> {
        self.record("user", &lookup)
    }

    async fn org(&self, lookup: OrgBudgetLookup<'_>) -> Result<(), LookupError> {
        self.record("org", &lookup)
    }

    async fn customer(&self, lookup: CustomerBudgetLookup<'_>) -> Result<(), LookupError> {
        self.record("customer", &lookup)
    }
}

fn run_fan_out(call: &FanOutCall) -> HelperResult {
    let collectors = budget_gauges(&call.noop_groups);
    let recorder = Recorder::default();
    let fan_out = RemainingBudgetFanOut::new(recorder.clone(), HELPER_TIMEOUT, &collectors);
    let request = RemainingBudgetRequest {
        team: call.team.as_deref(),
        team_alias: call.team_alias.as_deref(),
        key: call.key.as_deref(),
        key_alias: call.key_alias.as_deref(),
        user_id: call.user_id.as_deref(),
        org_id: call.org_id.as_deref(),
        end_user_id: call.end_user_id.as_deref(),
        response_cost: call.response_cost,
        metadata: &call.metadata,
    };
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| failure(&error))?;
    runtime.block_on(fan_out.increment_remaining_budget_metrics(request));
    Ok(json!(
        recorder
            .calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    ))
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Timeout { values } => timeout(values),
            Input::FanOut(call) => run_fan_out(call),
        }
    }
}
