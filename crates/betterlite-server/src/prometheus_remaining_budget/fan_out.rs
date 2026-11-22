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
use tokio::time::timeout;

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

type LookupOutcome = Result<(), LookupError>;

type LookupOutcomes = (
    LookupOutcome,
    LookupOutcome,
    LookupOutcome,
    LookupOutcome,
    LookupOutcome,
);

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
        match timeout(self.timeout, self.dispatch(lookups)).await {
            Ok(outcomes) => log_failures(outcomes),
            Err(_) => log_timeout(self.timeout),
        }
    }

    async fn dispatch(&self, lookups: BudgetLookups<'_>) -> LookupOutcomes {
        join5(
            self.lookups.key(lookups.key),
            self.lookups.team(lookups.team),
            self.lookups.user(lookups.user),
            self.lookups.org(lookups.org),
            self.lookups.customer(lookups.customer),
        )
        .await
    }
}

fn lookups_for(request: RemainingBudgetRequest<'_>) -> BudgetLookups<'_> {
    BudgetLookups {
        key: KeyBudgetLookup {
            user_api_key: request.key,
            user_api_key_alias: request.key_alias,
            response_cost: request.response_cost,
            key_max_budget: metadata_decimal(request.metadata, "user_api_key_max_budget"),
            key_spend: metadata_decimal(request.metadata, "user_api_key_spend"),
            carried: KeyBudgetSnapshot::from_metadata(request.metadata),
        },
        team: TeamBudgetLookup {
            user_api_team: request.team,
            user_api_team_alias: request.team_alias,
            team_spend: metadata_decimal(request.metadata, "user_api_key_team_spend"),
            team_max_budget: metadata_decimal(request.metadata, "user_api_key_team_max_budget"),
            response_cost: request.response_cost,
            carried: TeamBudgetSnapshot::from_metadata(request.metadata),
        },
        user: UserBudgetLookup {
            user_id: request.user_id,
            user_spend: metadata_decimal(request.metadata, "user_api_key_user_spend"),
            user_max_budget: metadata_decimal(request.metadata, "user_api_key_user_max_budget"),
            response_cost: request.response_cost,
            carried: UserBudgetSnapshot::from_metadata(request.metadata),
            user_email: metadata_text(request.metadata.get("user_api_key_user_email")),
        },
        org: OrgBudgetLookup {
            org_id: request.org_id,
            response_cost: request.response_cost,
            carried: OrgBudgetSnapshot::from_metadata(request.metadata),
            org_alias: metadata_text(request.metadata.get("user_api_key_org_alias")),
        },
        customer: CustomerBudgetLookup {
            end_user_id: request.end_user_id,
            response_cost: request.response_cost,
        },
    }
}

fn metadata_text(json: Option<&AgentJson>) -> Option<&str> {
    match json {
        Some(value) => value.as_str(),
        None => None,
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

fn log_timeout(limit: Duration) {
    tracing::debug!(
        "[Non-Blocking] Prometheus: per-request budget metric emission exceeded {:?}s under load; skipping (values are refreshed by the periodic budget-metrics cron job).",
        limit.as_secs_f64()
    );
}

fn log_failures(outcomes: LookupOutcomes) {
    log_failure("key", outcomes.0);
    log_failure("team", outcomes.1);
    log_failure("user", outcomes.2);
    log_failure("org", outcomes.3);
    log_failure("customer", outcomes.4);
}

fn log_failure(name: &str, outcome: LookupOutcome) {
    if let Err(error) = outcome {
        tracing::debug!("[Non-Blocking] Prometheus: Budget metric lookup {name} failed: {error}");
    }
}
