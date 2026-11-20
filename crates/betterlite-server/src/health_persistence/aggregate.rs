use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use indexmap::IndexMap;

use super::{AggregatedHealthResult, MAX_ERROR_CHARS, ModelParam, ModelParamIndex};

// the dict _aggregate_health_check_results builds: (model_id, model_name) to its counts, in first-insertion order
pub(super) type Aggregates = IndexMap<(Option<String>, String), AggregatedHealthResult>;

pub(super) fn count_healthy(
    aggregates: &mut Aggregates,
    index: &ModelParamIndex,
    healthy: &[JsonObject],
) {
    for endpoint in healthy {
        let models = endpoint_model(endpoint).and_then(|param| index.get(param));
        let Some(models) = models else {
            continue;
        };
        for model in models {
            let entry = aggregate_entry(aggregates, model);
            entry.healthy_count = entry.healthy_count.saturating_add(1);
        }
    }
}

// every unhealthy endpoint raises the count, but only the first non-empty error is kept for the key
pub(super) fn count_unhealthy(
    aggregates: &mut Aggregates,
    index: &ModelParamIndex,
    unhealthy: &[JsonObject],
) {
    for endpoint in unhealthy {
        let error = endpoint_error(endpoint);
        let models = endpoint_model(endpoint).and_then(|param| index.get(param));
        let Some(models) = models else {
            continue;
        };
        for model in models {
            let entry = aggregate_entry(aggregates, model);
            entry.unhealthy_count = entry.unhealthy_count.saturating_add(1);
            if entry.error_message.is_none() {
                entry.error_message = error
                    .as_ref()
                    .map(|text| text.chars().take(MAX_ERROR_CHARS).collect());
            }
        }
    }
}

fn aggregate_entry<'map>(
    aggregates: &'map mut Aggregates,
    model: &ModelParam,
) -> &'map mut AggregatedHealthResult {
    aggregates
        .entry((model.model_id.clone(), model.model_name.clone()))
        .or_insert_with(|| AggregatedHealthResult::from(model))
}

impl From<&ModelParam> for AggregatedHealthResult {
    fn from(model: &ModelParam) -> Self {
        Self {
            model_name: model.model_name.clone(),
            model_id: model.model_id.clone(),
            healthy_count: 0,
            unhealthy_count: 0,
            error_message: None,
        }
    }
}

// the endpoint's litellm_params.model, which is what a health check result files itself under
fn endpoint_model(endpoint: &JsonObject) -> Option<&str> {
    match endpoint.get("model") {
        Some(Json::Text(model)) => Some(model.as_str()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

// the first unhealthy endpoint's error, and only when it has one, cut as str(value)[:500] does
pub(super) fn first_error(unhealthy: &[JsonObject]) -> Option<String> {
    let endpoint = unhealthy.first()?;
    let error = endpoint_error(endpoint)?;
    Some(error.chars().take(MAX_ERROR_CHARS).collect())
}

// the endpoint's error, stringified, or nothing when it has no error value
fn endpoint_error(endpoint: &JsonObject) -> Option<String> {
    let error = match endpoint.get("error") {
        Some(Json::Text(text)) => text.clone(),
        Some(value @ (Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))) => {
            serde_json::to_string(value).unwrap_or_default()
        }
        Some(Json::Null) | None => return None,
    };
    (!error.is_empty()).then_some(error)
}
