use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use opentelemetry::trace::Tracer;

use crate::langfuse_observations::attributes::{
    ObservationAttributes, ObservationType, observation_attributes,
};
use crate::langfuse_observations::span::{ChildSpanStart, LangfuseObservation, start_child_span};

const GROUNDING_SPAN_NAME: &str = "vertex_ai_grounding_metadata";

// litellm.integrations.langfuse.langfuse.py.log_provider_specific_information_as_span; a hidden param that is not a mapping has nothing to read, where the source raises on .get
pub fn log_grounding_spans<T>(
    tracer: &T,
    parent: &LangfuseObservation<T::Span>,
    enrichments: &IndexMap<String, Json>,
) where
    T: Tracer,
{
    let hidden_params = match enrichments.get("hidden_params") {
        None | Some(Json::Null) => return,
        Some(Json::Object(hidden_params)) => hidden_params,
        Some(_) => {
            tracing::debug!("the hidden params are not a mapping, so no provider spans are logged");
            return;
        }
    };
    match hidden_params.get(GROUNDING_SPAN_NAME) {
        None | Some(Json::Null) => {}
        Some(Json::List(elements)) => {
            for element in elements {
                log_grounding_element(tracer, parent, element);
            }
        }
        Some(value) => end_grounding_span(tracer, parent, GROUNDING_SPAN_NAME, value),
    }
}

// a mapping element names each span after its key; any other element becomes one span of the whole element
fn log_grounding_element<T>(tracer: &T, parent: &LangfuseObservation<T::Span>, element: &Json)
where
    T: Tracer,
{
    if let Json::Object(pairs) = element {
        for (name, value) in pairs {
            end_grounding_span(tracer, parent, name, value);
        }
    } else {
        end_grounding_span(tracer, parent, GROUNDING_SPAN_NAME, element);
    }
}

// litellm.integrations.langfuse.langfuse.py._end_grounding_span; created and ended at once, with no own time window
pub fn end_grounding_span<T>(
    tracer: &T,
    parent: &LangfuseObservation<T::Span>,
    name: &str,
    value: &Json,
) where
    T: Tracer,
{
    let mut span = start_child_span(
        tracer,
        parent,
        ChildSpanStart {
            name: name.to_owned(),
            start_time: None,
            attributes: observation_attributes(&ObservationAttributes {
                observation_type: ObservationType::Span,
                input: Some(value),
                output: None,
                metadata: None,
                level: None,
                status_message: None,
                version: None,
                model: None,
                model_parameters: None,
                usage_details: None,
                cost_details: None,
                completion_start_time: None,
                prompt: None,
            }),
        },
    );
    span.end(None);
}
