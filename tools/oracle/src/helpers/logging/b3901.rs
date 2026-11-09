use betterlite_logging::langfuse_observations::attributes::{TraceAttributes, trace_attributes};
use betterlite_logging::langfuse_observations::identity::{
    resolve_observation_id, resolve_trace_id, to_unix_nanos,
};
use opentelemetry_sdk::trace::RandomIdGenerator;
use serde_json::json;

use crate::helpers::{Helper, HelperResult};

use identities::{parent_step, sampler_step};
use inputs::{Input, Step};
use projections::observation_projection;
use spans::child_step;
use values::pairs;

mod identities;
mod inputs;
mod projections;
mod spans;
mod values;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let results = input.steps.iter().map(|call| match call {
            Step::ResolveTraceId { input: requested } => json!(
                resolve_trace_id(requested.as_deref(), &RandomIdGenerator::default()).to_string()
            ),
            Step::ResolveObservationId { input: requested } => {
                json!(resolve_observation_id(requested.as_deref()).map(|id| id.to_string()))
            }
            Step::ToUnixNanos(moment) => json!(to_unix_nanos(moment.timestamp())),
            Step::TraceAttributes { fields } => {
                pairs(&trace_attributes(&TraceAttributes::from(fields)))
            }
            Step::ObservationAttributes { fields } => observation_projection(fields),
            Step::Sampler {
                rate,
                trace_id,
                attributes,
            } => sampler_step(*rate, trace_id, attributes),
            Step::ParentContext {
                trace_id,
                parent_observation_id,
                existing_trace,
                fallback_span_id,
            } => parent_step(
                trace_id,
                parent_observation_id.as_ref(),
                *existing_trace,
                fallback_span_id,
            ),
            Step::StartChildSpan(child) => child_step(child),
        });
        Ok(serde_json::Value::Array(results.collect()))
    }
}
