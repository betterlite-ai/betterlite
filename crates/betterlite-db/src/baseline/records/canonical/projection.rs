use std::collections::BTreeMap;

use betterlite_models::baseline_history::model::{BaselineObservation, Outcome};

use super::super::{BaselineAccountingRecord, DailyBaselineAttribution, DailyBaselineTarget};
use super::Canonical;
use super::datetime;
use crate::autorouter_turn_sql::transaction::AutoRouterTurnTransaction;

mod pricing;
mod usage;

pub(super) fn record(value: &BaselineAccountingRecord) -> Canonical {
    let mut root = Canonical::Object(BTreeMap::new());
    root.insert("scope", Canonical::Text(value.scope.clone()));
    root.insert("api_key", Canonical::Text(value.api_key.clone()));
    root.insert("session_id", Canonical::Text(value.session_id.clone()));
    root.insert("router_name", Canonical::Text(value.router_name.clone()));
    root.insert(
        "baseline_model",
        Canonical::Text(value.baseline_model.clone()),
    );
    root.insert("observation", observation(&value.observation));
    root.insert("pricing", pricing::pricing(&value.pricing));
    root.insert(
        "turn",
        value.turn.as_ref().map_or(Canonical::Null, turn_projection),
    );
    root.insert(
        "daily",
        value
            .daily
            .as_ref()
            .map_or(Canonical::Null, daily_projection),
    );
    root
}

fn observation(value: &BaselineObservation) -> Canonical {
    let mut root = Canonical::Object(BTreeMap::new());
    root.insert("version", Canonical::Integer(3));
    root.insert("request_id", Canonical::Text(value.request_id.clone()));
    root.insert("started_at", Canonical::Float(value.started_at));
    root.insert("available_at", Canonical::Float(value.available_at));
    root.insert(
        "outcome",
        Canonical::Text(outcome_text(value.outcome).to_owned()),
    );
    root.insert(
        "baseline_equivalent",
        Canonical::Bool(value.baseline_equivalent),
    );
    root.insert(
        "usage",
        value
            .usage
            .as_ref()
            .map_or(Canonical::Null, usage::usage_projection),
    );
    root.insert(
        "plan",
        value
            .plan
            .as_ref()
            .map_or(Canonical::Null, usage::plan_projection),
    );
    root.insert(
        "minimum_cache_tokens",
        Canonical::Integer(value.minimum_cache_tokens),
    );
    root.insert(
        "reason",
        value
            .reason
            .as_ref()
            .map_or(Canonical::Null, |text| Canonical::Text(text.clone())),
    );
    root
}

const fn outcome_text(value: Outcome) -> &'static str {
    match value {
        Outcome::Complete => "complete",
        Outcome::Uncertain => "uncertain",
        Outcome::ResponseCache => "response_cache",
    }
}

fn daily_projection(value: &DailyBaselineAttribution) -> Canonical {
    let mut root = Canonical::Object(BTreeMap::new());
    root.insert("date", Canonical::Text(value.date.clone()));
    root.insert("api_key", Canonical::Text(value.api_key.clone()));
    for (key, text) in [
        ("model", &value.model),
        ("custom_llm_provider", &value.custom_llm_provider),
        ("model_group", &value.model_group),
        ("endpoint", &value.endpoint),
        ("mcp_namespaced_tool_name", &value.mcp_namespaced_tool_name),
    ] {
        root.insert(
            key,
            text.as_ref()
                .map_or(Canonical::Null, |inner| Canonical::Text(inner.clone())),
        );
    }
    root.insert(
        "targets",
        Canonical::List(value.targets.iter().map(target).collect()),
    );
    root
}

fn target(value: &DailyBaselineTarget) -> Canonical {
    let mut root = Canonical::Object(BTreeMap::new());
    root.insert(
        "entity",
        Canonical::Text(value.entity.names().name.to_owned()),
    );
    root.insert(
        "entity_id",
        value
            .entity_id
            .as_ref()
            .map_or(Canonical::Null, |text| Canonical::Text(text.clone())),
    );
    root
}

fn turn_projection(value: &AutoRouterTurnTransaction) -> Canonical {
    let mut root = Canonical::Object(BTreeMap::new());
    root.insert("api_key", Canonical::Text(value.api_key.clone()));
    root.insert("session_id", Canonical::Text(value.session_id.clone()));
    root.insert("router_name", Canonical::Text(value.router_name.clone()));
    root.insert("router_type", Canonical::Text(value.router_type.clone()));
    root.insert("model", Canonical::Text(value.model.clone()));
    root.insert(
        "turn_at",
        Canonical::Text(datetime::turn_time(&value.turn_at)),
    );
    root.insert("total_tokens", Canonical::Integer(value.total_tokens));
    root.insert("spend", Canonical::from(value.spend));
    root.insert("saved_spend", Canonical::from(value.saved_spend));
    root.insert("classifier_cost", Canonical::from(value.classifier_cost));
    root.insert("covered", Canonical::Bool(value.covered.0));
    root.insert("cache_hit", Canonical::Bool(value.cache_hit.0));
    root.insert(
        "cache_ttl_seconds",
        Canonical::from(value.cache_ttl_seconds),
    );
    root.insert("cache_touched", Canonical::Bool(value.cache_touched.0));
    for (key, text) in [
        ("tier", &value.tier),
        ("baseline_model", &value.baseline_model),
    ] {
        root.insert(
            key,
            text.as_ref()
                .map_or(Canonical::Null, |inner| Canonical::Text(inner.clone())),
        );
    }
    insert_savings_estimate(&mut root, value);
    root.insert("user_id", Canonical::Text(value.user_id.clone()));
    root
}

fn insert_savings_estimate(root: &mut Canonical, value: &AutoRouterTurnTransaction) {
    root.insert(
        "savings_estimated_turns",
        Canonical::Integer(value.savings_estimated_turns),
    );
    root.insert(
        "savings_estimated_actual_spend",
        Canonical::from(value.savings_estimated_actual_spend),
    );
    root.insert(
        "savings_estimated_saved_spend",
        Canonical::from(value.savings_estimated_saved_spend),
    );
}
