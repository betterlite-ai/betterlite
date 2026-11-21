use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_core::agents::json::AgentJson;
use betterlite_core::proxy::carried_budget_state::{
    BudgetSnapshot as _, KeyBudgetSnapshot, OrgBudgetSnapshot, TeamBudgetSnapshot,
    UserBudgetSnapshot,
};
use betterlite_logging::prometheus::budget_gauges::customer_gate::budget_gauges_are_noop;
use betterlite_logging::prometheus::budget_gauges::gauge::BudgetGauges;
use betterlite_prometheus::collector_factory::collector::MetricLike;
use futures_util::future::join5;
use rust_decimal::Decimal;

use super::lookups::{
    CustomerBudgetLookup, KeyBudgetLookup, LookupError, OrgBudgetLookup, RequestBudgetLookups,
    TeamBudgetLookup, UserBudgetLookup,
};

#[derive(Clone, Copy)]
pub struct RemainingBudgetRequest<'request> {
    pub team: Option<&'request str>,
    pub team_alias: Option<&'request str>,
    pub key: Option<&'request str>,
    pub key_alias: Option<&'request str>,
    pub user_id: Option<&'request str>,
    pub org_id: Option<&'request str>,
    pub end_user_id: Option<&'request str>,
    pub response_cost: Decimal,
    pub metadata: &'request BTreeMap<String, AgentJson>,
}

pub struct RemainingBudgetFanOut<L> {
    lookups: L,
    timeout: Duration,
    enabled: bool,
}

struct BudgetLookups<'request> {
    key: KeyBudgetLookup<'request>,
    team: TeamBudgetLookup<'request>,
    user: UserBudgetLookup<'request>,
    org: OrgBudgetLookup<'request>,
    customer: CustomerBudgetLookup<'request>,
}

impl<L: RequestBudgetLookups> RemainingBudgetFanOut<L> {
    // the gauges never change after registration, so LiteLLM's per-call no-op check runs once here
    #[must_use]
    pub fn new<Gauge>(
        lookups: L,
        timeout: Duration,
        collectors: &BudgetGauges<MetricLike<Gauge>>,
    ) -> Self {
        let gauges_are_noop = matches!(&collectors.team.remaining.collector, MetricLike::NoOp(_))
            && matches!(&collectors.key.remaining.collector, MetricLike::NoOp(_))
            && matches!(&collectors.user.remaining.collector, MetricLike::NoOp(_))
            && matches!(&collectors.org.remaining.collector, MetricLike::NoOp(_))
            && budget_gauges_are_noop(&collectors.customer);
        Self {
            lookups,
            timeout,
            enabled: !gauges_are_noop,
        }
    }

    // ast-grep-ignore: no-long-names - LiteLLM's method name, so B8561's caller reads the same as its Python
    pub async fn increment_remaining_budget_metrics(&self, request: RemainingBudgetRequest<'_>) {
        if !self.enabled {
            return;
        }
        let lookups = lookups_for(request);
        match tokio::time::timeout(self.timeout, self.dispatch(lookups)).await {
            Ok(results) => log_failures(results),
            Err(_) => log_timeout(self.timeout),
        }
    }

    async fn dispatch(&self, lookups: BudgetLookups<'_>) -> [Result<(), LookupError>; 5] {
        let (key, team, user, org, customer) = join5(
            self.lookups.key(lookups.key),
            self.lookups.team(lookups.team),
            self.lookups.user(lookups.user),
            self.lookups.org(lookups.org),
            self.lookups.customer(lookups.customer),
        )
        .await;
        [key, team, user, org, customer]
    }
}

fn lookups_for(request: RemainingBudgetRequest<'_>) -> BudgetLookups<'_> {
    let metadata = request.metadata;
    BudgetLookups {
        key: key_lookup(request, metadata),
        team: team_lookup(request, metadata),
        user: user_lookup(request, metadata),
        org: org_lookup(request, metadata),
        customer: customer_lookup(request),
    }
}

fn key_lookup<'request>(
    request: RemainingBudgetRequest<'request>,
    metadata: &'request BTreeMap<String, AgentJson>,
) -> KeyBudgetLookup<'request> {
    KeyBudgetLookup {
        key: request.key,
        key_alias: request.key_alias,
        response_cost: request.response_cost,
        max_budget: metadata_decimal(metadata, "user_api_key_max_budget"),
        spend: metadata_decimal(metadata, "user_api_key_spend"),
        carried: KeyBudgetSnapshot::from_metadata(metadata),
    }
}

fn team_lookup<'request>(
    request: RemainingBudgetRequest<'request>,
    metadata: &'request BTreeMap<String, AgentJson>,
) -> TeamBudgetLookup<'request> {
    TeamBudgetLookup {
        team: request.team,
        team_alias: request.team_alias,
        spend: metadata_decimal(metadata, "user_api_key_team_spend"),
        max_budget: metadata_decimal(metadata, "user_api_key_team_max_budget"),
        response_cost: request.response_cost,
        carried: TeamBudgetSnapshot::from_metadata(metadata),
    }
}

fn user_lookup<'request>(
    request: RemainingBudgetRequest<'request>,
    metadata: &'request BTreeMap<String, AgentJson>,
) -> UserBudgetLookup<'request> {
    UserBudgetLookup {
        user_id: request.user_id,
        spend: metadata_decimal(metadata, "user_api_key_user_spend"),
        max_budget: metadata_decimal(metadata, "user_api_key_user_max_budget"),
        response_cost: request.response_cost,
        carried: UserBudgetSnapshot::from_metadata(metadata),
        user_email: metadata
            .get("user_api_key_user_email")
            .and_then(AgentJson::as_str),
    }
}

fn org_lookup<'request>(
    request: RemainingBudgetRequest<'request>,
    metadata: &'request BTreeMap<String, AgentJson>,
) -> OrgBudgetLookup<'request> {
    OrgBudgetLookup {
        org_id: request.org_id,
        response_cost: request.response_cost,
        carried: OrgBudgetSnapshot::from_metadata(metadata),
        org_alias: metadata
            .get("user_api_key_org_alias")
            .and_then(AgentJson::as_str),
    }
}

fn customer_lookup(request: RemainingBudgetRequest<'_>) -> CustomerBudgetLookup<'_> {
    CustomerBudgetLookup {
        end_user_id: request.end_user_id,
        response_cost: request.response_cost,
    }
}

fn metadata_decimal(metadata: &BTreeMap<String, AgentJson>, name: &str) -> Option<Decimal> {
    let value = metadata.get(name)?;
    let number = value.as_number()?;
    decimal_text(&number.to_string())
}

// a JSON number may arrive in exponent notation, which Decimal's own parser refuses
fn decimal_text(text: &str) -> Option<Decimal> {
    text.parse::<Decimal>()
        .ok()
        .or_else(|| Decimal::from_scientific(text).ok())
}

fn log_timeout(timeout: Duration) {
    tracing::debug!(
        "[Non-Blocking] Prometheus: per-request budget metric emission exceeded {:?}s under load; skipping (values are refreshed by the periodic budget-metrics cron job).",
        timeout.as_secs_f64()
    );
}

fn log_failures(results: [Result<(), LookupError>; 5]) {
    let names = ["key", "team", "user", "org", "customer"];
    for (name, result) in names.into_iter().zip(results) {
        if let Err(error) = result {
            tracing::debug!(
                "[Non-Blocking] Prometheus: Budget metric lookup {name} failed: {error}"
            );
        }
    }
}
