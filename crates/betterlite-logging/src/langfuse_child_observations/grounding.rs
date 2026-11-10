use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use opentelemetry::trace::Tracer;

use super::child_spans::{ChildSpans, span_attributes};

const GROUNDING_SPAN_NAME: &str = "vertex_ai_grounding_metadata";

// litellm.integrations.langfuse.langfuse.py.log_provider_specific_information_as_span; a hidden param that is not a mapping has nothing to read, where the source raises on .get
pub fn log_grounding_spans<T>(spans: &ChildSpans<'_, T>, enrichments: &IndexMap<String, Json>)
where
    T: Tracer,
{
    let Some(grounding) = grounding_value(enrichments) else {
        return;
    };
    let Json::List(elements) = grounding else {
        end_grounding_span(spans, GROUNDING_SPAN_NAME, grounding);
        return;
    };
    for element in elements {
        let Json::Object(pairs) = element else {
            end_grounding_span(spans, GROUNDING_SPAN_NAME, element);
            continue;
        };
        for (name, value) in pairs {
            end_grounding_span(spans, name, value);
        }
    }
}

fn grounding_value(enrichments: &IndexMap<String, Json>) -> Option<&Json> {
    let hidden_params = match enrichments.get("hidden_params") {
        None | Some(Json::Null) => return None,
        Some(Json::Object(hidden_params)) => hidden_params,
        Some(_) => {
            tracing::debug!("the hidden params are not a mapping, so no provider spans are logged");
            return None;
        }
    };
    match hidden_params.get(GROUNDING_SPAN_NAME) {
        None | Some(Json::Null) => None,
        Some(value) => Some(value),
    }
}

// litellm.integrations.langfuse.langfuse.py._end_grounding_span; created and ended at once, with no own time window
pub fn end_grounding_span<T>(spans: &ChildSpans<'_, T>, name: &str, value: &Json)
where
    T: Tracer,
{
    let attributes = span_attributes(Some(value), None, None);
    let mut span = spans.start(name, None, attributes);
    span.end(None);
}
