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
    fields.insert(
        "total_tokens".to_owned(),
        Json::Number(Number::from(counts.0)),
    );
    fields.insert(
        "prompt_tokens".to_owned(),
        Json::Number(Number::from(counts.1)),
    );
    fields.insert(
        "completion_tokens".to_owned(),
        Json::Number(Number::from(counts.2)),
    );
    if let Some(hit) = cache_hit {
        fields.insert("cache_hit".to_owned(), Json::Bool(hit));
    }
    if let Some(value) = usage {
        let mut metadata = IndexMap::new();
        metadata.insert("usage_object".to_owned(), value.clone());
        fields.insert("metadata".to_owned(), Json::Object(metadata));
    }
    let text = serde_json::to_string(&Json::Object(fields))
        .map_err(|error| helper_error("ValueError", &error.to_string()))?;
    serde_json::from_str(&text).map_err(|error| helper_error("ValueError", &error.to_string()))
}

fn base_document() -> IndexMap<String, Json> {
    let entries = [
        ("id", Json::Text("payload".to_owned())),
        ("trace_id", Json::Text("trace".to_owned())),
        ("session_id", Json::Text("session".to_owned())),
        ("call_type", Json::Text("completion".to_owned())),
        ("status", Json::Text("success".to_owned())),
        ("status_fields", status_fields()),
        ("startTime", Json::Number(Number::from(0_i64))),
        ("endTime", Json::Number(Number::from(0_i64))),
        ("completionStartTime", Json::Number(Number::from(0_i64))),
        ("response_time", Json::Number(Number::from(0_i64))),
        ("model_map_information", Json::Object(IndexMap::new())),
        ("model", Json::Text("gpt-4o".to_owned())),
        ("api_base", Json::Text(String::new())),
        ("metadata", Json::Object(IndexMap::new())),
        ("saved_cache_cost", Json::Number(Number::from(0_i64))),
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
        (
            "llm_api_status".to_owned(),
            Json::Text("success".to_owned()),
        ),
        (
            "guardrail_status".to_owned(),
            Json::Text("not_run".to_owned()),
        ),
    ]))
}
