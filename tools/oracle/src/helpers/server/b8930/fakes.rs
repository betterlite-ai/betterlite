use std::str::FromStr as _;
use std::sync::{Arc, Mutex};

use betterlite_core::cost::recording::{RecordInput, RecordResponse};
use betterlite_core::cost::usage::BillingUsage;
use betterlite_logging::fields::usage::Usage;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_server::stream_usage_cost::{StreamCostCalculator, StreamCostRecorder};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde_json::Number;

use super::DEBUG_KEY;

pub(super) struct FakeRecorder {
    pub(super) cost: Option<Decimal>,
    pub(super) observed: Arc<Mutex<Vec<Json>>>,
}

impl StreamCostRecorder for FakeRecorder {
    fn response_cost(&self, log: &mut LoggingState, input: RecordInput<'_>) -> Option<Decimal> {
        let Some(usage) = chat_usage(&input) else {
            return None;
        };
        push(
            &self.observed,
            call_json("recorder", &log.model, None, &usage),
        );
        match self.cost {
            Some(cost) => Some(cost),
            None => {
                log.model_call_details
                    .insert(DEBUG_KEY.to_owned(), failure_debug());
                None
            }
        }
    }
}

fn chat_usage(input: &RecordInput<'_>) -> Option<Usage> {
    let RecordResponse::Typed { response, .. } = &input.response else {
        return None;
    };
    let BillingUsage::Chat(usage) = &response.usage else {
        return None;
    };
    Some((*usage).clone())
}

pub(super) struct FakeCalculator {
    pub(super) cost: Option<Decimal>,
    pub(super) observed: Arc<Mutex<Vec<Json>>>,
}

impl StreamCostCalculator for FakeCalculator {
    fn completion_cost(
        &self,
        usage: &Usage,
        model_name: &str,
        service_tier: Option<&str>,
    ) -> Option<Decimal> {
        push(
            &self.observed,
            call_json("calculator", model_name, service_tier, usage),
        );
        self.cost
    }
}

pub(super) fn price(value: Option<&Json>) -> Option<Decimal> {
    match value? {
        Json::Number(number) => Decimal::from_str(&number.to_string()).ok(),
        Json::Text(text) => Decimal::from_str(text).ok(),
        Json::Null | Json::Bool(_) | Json::List(_) | Json::Object(_) => None,
    }
}

pub(super) fn take(observed: &Mutex<Vec<Json>>) -> Vec<Json> {
    let mut entries = match observed.lock() {
        Ok(found) => found,
        Err(poisoned) => poisoned.into_inner(),
    };
    entries.split_off(0)
}

fn push(observed: &Mutex<Vec<Json>>, call: Json) {
    let mut entries = match observed.lock() {
        Ok(found) => found,
        Err(poisoned) => poisoned.into_inner(),
    };
    entries.push(call);
}

fn call_json(service: &str, model: &str, service_tier: Option<&str>, usage: &Usage) -> Json {
    let mut fields = IndexMap::new();
    fields.insert("service".to_owned(), Json::Text(service.to_owned()));
    fields.insert("model".to_owned(), Json::Text(model.to_owned()));
    fields.insert(
        "service_tier".to_owned(),
        match service_tier {
            Some(tier) => Json::Text(tier.to_owned()),
            None => Json::Null,
        },
    );
    fields.insert("usage".to_owned(), usage_json(usage));
    Json::Object(fields)
}

fn usage_json(usage: &Usage) -> Json {
    let mut fields = IndexMap::new();
    fields.insert(
        "prompt_tokens".to_owned(),
        Json::Number(Number::from(usage.prompt_tokens)),
    );
    fields.insert(
        "completion_tokens".to_owned(),
        Json::Number(Number::from(usage.completion_tokens)),
    );
    fields.insert(
        "total_tokens".to_owned(),
        Json::Number(Number::from(usage.total_tokens)),
    );
    fields.insert(
        "prompt_tokens_details".to_owned(),
        details_json(usage.prompt_tokens_details.as_ref()),
    );
    fields.insert(
        "completion_tokens_details".to_owned(),
        details_json(usage.completion_tokens_details.as_ref()),
    );
    Json::Object(fields)
}

fn details_json(details: Option<&IndexMap<String, Json>>) -> Json {
    match details {
        Some(fields) => drop_nulls(Json::Object(fields.clone())),
        None => Json::Null,
    }
}

// LiteLLM dumps its detail models with exclude_none, so both sides compare only the reported counts.
fn drop_nulls(value: Json) -> Json {
    match value {
        Json::Object(fields) => Json::Object(
            fields
                .into_iter()
                .map(|(name, value)| (name, drop_nulls(value)))
                .filter(|(_, value)| !matches!(value, Json::Null))
                .collect(),
        ),
        Json::List(items) => Json::List(items.into_iter().map(drop_nulls).collect()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => value,
    }
}

fn failure_debug() -> Json {
    let mut fields = IndexMap::new();
    fields.insert(
        "error_str".to_owned(),
        Json::Text("scripted failure".to_owned()),
    );
    Json::Object(fields)
}
