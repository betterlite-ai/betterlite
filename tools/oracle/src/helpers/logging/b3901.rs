use betterlite_logging::langfuse_observations::identity::{
    resolve_observation_id, resolve_trace_id, to_unix_nanos,
};
use opentelemetry_sdk::trace::RandomIdGenerator;
use serde_json::json;

use crate::helpers::{Helper, HelperResult};

use identities::{parent_step, sampler_step};
use inputs::{Input, Step};
use projections::{observation_projection, timestamp, trace_projection};
use spans::child_step;

mod identities;
mod inputs;
mod projections;
mod scripted;
mod spans;
mod values;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut results = Vec::with_capacity(input.steps.len());
        for step in &input.steps {
            results.push(run_step(step)?);
        }
        Ok(serde_json::Value::Array(results))
    }
}

fn run_step(step: &Step) -> Result<serde_json::Value, Box<crate::helpers::HelperError>> {
    match step {
        Step::ResolveTraceId { input } => Ok(json!(
            resolve_trace_id(input.as_deref(), &RandomIdGenerator::default()).to_string()
        )),
        Step::ResolveObservationId { input } => Ok(json!(
            resolve_observation_id(input.as_deref()).map(|id| id.to_string())
        )),
        Step::ToUnixNanos { seconds, datetime } => Ok(json!(to_unix_nanos(timestamp(
            *seconds,
            datetime.as_deref()
        )?))),
        Step::TraceAttributes { fields } => Ok(trace_projection(fields)),
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
            parent_observation_id.as_deref(),
            *existing_trace,
            fallback_span_id,
        ),
        Step::StartChildSpan { .. } => child_step(step),
    }
}
