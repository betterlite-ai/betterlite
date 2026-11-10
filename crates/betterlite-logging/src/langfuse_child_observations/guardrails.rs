use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use opentelemetry::trace::Tracer;

use crate::fields::payload::StandardLoggingPayload;
use crate::fields::payload::records::GuardrailInformation;
use crate::langfuse_observations::identity::Timestamp;

use super::child_spans::{ChildSpans, span_attributes};

const GUARDRAIL_SPAN_NAME: &str = "guardrail";

// litellm.integrations.langfuse.langfuse.py._log_guardrail_information_as_span; the typed payload turns the source's non-list and non-dict guards into decode failures (docs/compatibility.md)
pub fn log_guardrail_spans<T>(
    spans: &ChildSpans<'_, T>,
    standard_logging_payload: Option<&StandardLoggingPayload>,
) where
    T: Tracer,
{
    let Some(entries) = standard_logging_payload
        .and_then(|payload| payload.guardrail_information.as_deref())
        .filter(|entries| !entries.is_empty())
    else {
        tracing::debug!("the standard logging payload carries no guardrail information to log");
        return;
    };
    for entry in entries {
        log_guardrail_span(spans, entry);
    }
}

fn log_guardrail_span<T>(spans: &ChildSpans<'_, T>, entry: &GuardrailInformation)
where
    T: Tracer,
{
    let metadata = guardrail_metadata(entry);
    let mut span = spans.start(
        GUARDRAIL_SPAN_NAME,
        entry
            .start_time
            .as_ref()
            .and_then(|value| value.as_ref())
            .map(|value| Timestamp::Seconds(value.0)),
        span_attributes(
            entry
                .guardrail_request
                .as_ref()
                .and_then(|value| value.as_ref()),
            entry
                .guardrail_response
                .as_ref()
                .and_then(|value| value.as_ref()),
            Some(&metadata),
        ),
    );
    span.end(
        entry
            .end_time
            .as_ref()
            .and_then(|value| value.as_ref())
            .map(|value| Timestamp::Seconds(value.0)),
    );
}

// the source maps masked_entity_count onto guardrail_masked_entity_count, a name the guardrail record does not use
fn guardrail_metadata(entry: &GuardrailInformation) -> Json {
    let mut fields = IndexMap::new();
    if let Some(name) = entry
        .guardrail_name
        .as_ref()
        .and_then(|name| name.as_deref())
    {
        fields.insert("guardrail_name".to_owned(), Json::Text(name.to_owned()));
    }
    if let Some(mode) = entry.guardrail_mode.as_ref().and_then(|mode| mode.as_ref()) {
        fields.insert("guardrail_mode".to_owned(), mode.clone());
    }
    if let Some(counts) = entry
        .masked_entity_count
        .as_ref()
        .and_then(|counts| counts.as_ref())
    {
        let by_kind: IndexMap<String, Json> = counts
            .iter()
            .map(|(kind, count)| (kind.clone(), Json::Number(serde_json::Number::from(*count))))
            .collect();
        fields.insert(
            "guardrail_masked_entity_count".to_owned(),
            Json::Object(by_kind),
        );
    }
    Json::Object(fields)
}
