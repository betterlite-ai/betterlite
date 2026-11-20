use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::HelperError;
use indexmap::IndexMap;
use serde_json::Number;

use super::helper_error;

pub(super) fn payload(
    response_cost: Option<&Json>,
    counts: (i64, i64, i64),
    cache_hit: Option<bool>,
    usage: Option<&Json>,
) -> Result<StandardLoggingPayload, Box<HelperError>> {
    let mut fields = base_document();
    let cost = response_cost.map_or_else(|| Json::Number(Number::from(0_i64)), Json::clone);
    fields.insert("response_cost".to_owned(), cost);
    fields.insert("total_tokens".to_owned(), number(counts.0));
    fields.insert("prompt_tokens".to_owned(), number(counts.1));
    fields.insert("completion_tokens".to_owned(), number(counts.2));
    if let Some(hit) = cache_hit {
        fields.insert("cache_hit".to_owned(), Json::Bool(hit));
    }
    if let Some(usage) = usage {
        let mut metadata = IndexMap::new();
        metadata.insert("usage_object".to_owned(), usage.clone());
        fields.insert("metadata".to_owned(), Json::Object(metadata));
    }
    let text = serde_json::to_string(&Json::Object(fields))
        .map_err(|error| helper_error("ValueError", &error.to_string()))?;
    serde_json::from_str(&text).map_err(|error| helper_error("ValueError", &error.to_string()))
}

fn base_document() -> IndexMap<String, Json> {
    let entries = [
        ("id", text("payload")),
        ("trace_id", text("trace")),
        ("session_id", text("session")),
        ("call_type", text("completion")),
        ("status", text("success")),
        ("status_fields", status_fields()),
        ("startTime", number(0)),
        ("endTime", number(0)),
        ("completionStartTime", number(0)),
        ("response_time", number(0)),
        ("model_map_information", Json::Object(IndexMap::new())),
        ("model", text("gpt-4o")),
        ("api_base", text("")),
        ("metadata", Json::Object(IndexMap::new())),
        ("saved_cache_cost", number(0)),
        ("request_tags", Json::List(Vec::new())),
        ("model_parameters", Json::Object(IndexMap::new())),
        ("hidden_params", Json::Object(IndexMap::new())),
    ];
    entries
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect()
}

fn status_fields() -> Json {
    Json::Object(IndexMap::from([
        ("llm_api_status".to_owned(), text("success")),
        ("guardrail_status".to_owned(), text("not_run")),
    ]))
}

fn number(value: i64) -> Json {
    Json::Number(Number::from(value))
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}
