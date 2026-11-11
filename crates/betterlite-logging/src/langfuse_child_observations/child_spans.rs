use betterlite_models::decisions::Json;
use opentelemetry::KeyValue;
use opentelemetry::trace::Tracer;

use crate::langfuse_observations::attributes::{
    ObservationAttributes, ObservationType, observation_attributes,
};
use crate::langfuse_observations::identity::Timestamp;
use crate::langfuse_observations::span::{ChildSpanStart, LangfuseObservation, start_child_span};

// litellm's child observations hang under one generation: the tracing client and the parent observation it starts from
pub struct ChildSpans<'target, T>
where
    T: Tracer,
{
    pub tracer: &'target T,
    pub parent: &'target LangfuseObservation<T::Span>,
}

impl<T> ChildSpans<'_, T>
where
    T: Tracer,
{
    pub(crate) fn start(
        &self,
        name: &str,
        start_time: Option<Timestamp>,
        attributes: Vec<KeyValue>,
    ) -> LangfuseObservation<T::Span> {
        start_child_span(
            self.tracer,
            self.parent,
            ChildSpanStart {
                name: name.to_owned(),
                start_time,
                attributes,
            },
        )
    }
}

// litellm's observation_attributes(observation_type="span") for a child observation; these are all the fields it sets
pub(crate) fn span_attributes(
    input: Option<&Json>,
    output: Option<&Json>,
    metadata: Option<&Json>,
) -> Vec<KeyValue> {
    observation_attributes(&ObservationAttributes {
        observation_type: ObservationType::Span,
        input,
        output,
        metadata,
        level: None,
        status_message: None,
        version: None,
        model: None,
        model_parameters: None,
        usage_details: None,
        cost_details: None,
        completion_start_time: None,
        prompt: None,
    })
}
