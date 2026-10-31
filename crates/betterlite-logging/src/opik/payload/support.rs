use betterlite_models::decisions::Json;
use chrono::SecondsFormat;
use indexmap::IndexMap;

use crate::opik::uuid7::create_uuid7;

use super::{PayloadContent, PayloadWindow, SpanContext, SpanDetails};

pub(super) struct SpanIdentity {
    pub(super) model: String,
    pub(super) name: String,
}

pub(super) fn span_identity(response: &IndexMap<String, Json>) -> SpanIdentity {
    let model = string_or(response, "model", "unknown-model");
    let name = format!(
        "{model}_{}_{}",
        string_or(response, "object", "unknown-object"),
        integer_or(response, "created", 0)
    );
    SpanIdentity { model, name }
}

pub(super) fn span_entries(
    context: SpanContext<'_>,
    window: PayloadWindow,
    identity: SpanIdentity,
    details: SpanDetails<'_>,
    usage: IndexMap<String, Json>,
) -> [(&'static str, Json); 11] {
    let start_time = window.start.to_rfc3339_opts(SecondsFormat::Millis, true);
    let end_time = window.end.to_rfc3339_opts(SecondsFormat::Millis, true);
    [
        ("id", Json::Text(create_uuid7())),
        ("trace_id", Json::Text(context.trace_id.to_owned())),
        ("name", Json::Text(identity.name)),
        ("type", Json::Text("llm".to_owned())),
        ("model", Json::Text(identity.model)),
        ("start_time", Json::Text(start_time)),
        ("end_time", Json::Text(end_time)),
        (
            "parent_span_id",
            context.parent_span_id.map_or(Json::Null, Json::Text),
        ),
        (
            "provider",
            details
                .provider
                .map_or(Json::Null, |value| Json::Text(value.to_owned())),
        ),
        ("total_cost", details.cost.unwrap_or(Json::Null)),
        ("usage", Json::Object(usage)),
    ]
}

pub(super) fn common_payload(
    project: &str,
    window: PayloadWindow,
    content: PayloadContent,
) -> IndexMap<String, Json> {
    let start_time = window.start.to_rfc3339_opts(SecondsFormat::Millis, true);
    let end_time = window.end.to_rfc3339_opts(SecondsFormat::Millis, true);
    let mut payload = IndexMap::new();
    extend_payload(
        &mut payload,
        [
            ("project_name", Json::Text(project.to_owned())),
            ("start_time", Json::Text(start_time)),
            ("end_time", Json::Text(end_time)),
            ("input", content.input),
            ("output", content.output),
            ("metadata", Json::Object(content.metadata)),
            ("tags", Json::List(content.tags)),
        ],
    );
    payload
}

pub(super) fn extend_payload(
    payload: &mut IndexMap<String, Json>,
    entries: impl IntoIterator<Item = (&'static str, Json)>,
) {
    for (key, value) in entries {
        payload.insert(key.to_owned(), value);
    }
}

pub(super) fn remove_nulls(mut payload: IndexMap<String, Json>) -> IndexMap<String, Json> {
    payload.retain(|_, value| !matches!(value, Json::Null));
    payload
}

pub(super) fn string_or(source: &IndexMap<String, Json>, key: &str, default: &str) -> String {
    match source.get(key) {
        Some(Json::Text(value)) => value.clone(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => default.to_owned(),
    }
}

pub(super) fn integer_or(source: &IndexMap<String, Json>, key: &str, default: i64) -> i64 {
    match source.get(key) {
        Some(Json::Number(value)) => value.as_i64().unwrap_or(default),
        Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_))
        | None => default,
    }
}

pub(super) fn object_field<'source>(
    source: &'source IndexMap<String, Json>,
    key: &str,
) -> Option<&'source IndexMap<String, Json>> {
    match source.get(key)? {
        Json::Object(value) => Some(value),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}
