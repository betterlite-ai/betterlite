use betterlite_logging::langfuse_observations::attributes::{
    ObservationAttributes, ObservationType, PromptLink, TraceAttributes, observation_attributes,
    trace_attributes,
};
use betterlite_logging::langfuse_observations::identity::{
    Timestamp, resolve_observation_id, resolve_trace_id, to_unix_nanos,
};
use betterlite_logging::langfuse_observations::requested_ids::{
    RequestedIdGenerator, RequestedIds,
};
use betterlite_logging::langfuse_observations::sampler::TraceIdHashSampler;
use betterlite_logging::langfuse_observations::span::{
    ChildSpanStart, GenerationStart, LangfuseObservation, parent_context, start_child_span,
    start_generation,
};
use betterlite_models::decisions::Json;
use chrono::DateTime;
use opentelemetry::trace::{
    SpanContext, SpanId, SpanKind, TraceContextExt as _, TraceFlags, TraceId, TraceState,
};
use opentelemetry::{KeyValue, StringValue, Value};
use opentelemetry_sdk::trace::{
    IdGenerator, RandomIdGenerator, SamplingDecision, ShouldSample as _,
};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{Helper, HelperError, HelperResult};

use scripted::{ScriptedSpan, ScriptedTracer, system_nanos};

mod scripted;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    steps: Vec<Step>,
}

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
enum Step {
    ResolveTraceId {
        input: Option<String>,
    },
    ResolveObservationId {
        input: Option<String>,
    },
    ToUnixNanos {
        seconds: Option<Decimal>,
        datetime: Option<String>,
    },
    TraceAttributes {
        fields: TraceFields,
    },
    ObservationAttributes {
        fields: ObservationFields,
    },
    RequestedIds {
        trace_id: String,
        span_id: String,
    },
    Sampler {
        rate: f64,
        trace_id: String,
        attributes: Vec<(String, Json)>,
    },
    ParentContext {
        trace_id: String,
        parent_observation_id: Option<String>,
        existing_trace: bool,
        fallback_span_id: String,
    },
    Observation {
        span_id: String,
        public: Option<bool>,
        seconds: Option<Decimal>,
        datetime: Option<String>,
    },
    StartGeneration {
        trace_id: String,
        observation_id: Option<String>,
        parent_observation_id: Option<String>,
        existing_trace: bool,
        name: String,
        seconds: Option<Decimal>,
        datetime: Option<String>,
        public: Option<bool>,
        attributes: Vec<(String, Json)>,
        span_id: String,
    },
    StartChildSpan {
        parent_trace_id: String,
        parent_span_id: String,
        parent_public: Option<bool>,
        name: String,
        seconds: Option<Decimal>,
        datetime: Option<String>,
        attributes: Vec<(String, Json)>,
        child_span_id: String,
    },
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct TraceFields {
    name: Option<Json>,
    user_id: Option<Json>,
    session_id: Option<Json>,
    version: Option<Json>,
    release: Option<Json>,
    tags: Option<Json>,
    metadata: Option<Json>,
    public: Option<bool>,
    input: Option<Json>,
    output: Option<Json>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ObservationFields {
    observation_type: String,
    input: Option<Json>,
    output: Option<Json>,
    metadata: Option<Json>,
    level: Option<Json>,
    status_message: Option<Json>,
    version: Option<Json>,
    model: Option<Json>,
    model_parameters: Option<Json>,
    usage_details: Option<Json>,
    cost_details: Option<Json>,
    completion_start_time: Option<String>,
    prompt: Option<PromptFields>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PromptFields {
    name: String,
    version: Option<i64>,
    is_fallback: bool,
}

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

fn run_step(step: &Step) -> Result<serde_json::Value, Box<HelperError>> {
    match step {
        Step::ResolveTraceId { input } => Ok(json!(
            resolve_trace_id(input.as_deref(), &RandomIdGenerator::default()).to_string()
        )),
        Step::ResolveObservationId { input } => Ok(json!(
            resolve_observation_id(input.as_deref()).map(|id| id.to_string())
        )),
        Step::ToUnixNanos { seconds, datetime } => nanos(*seconds, datetime.as_deref()),
        Step::TraceAttributes { fields } => Ok(trace_projection(fields)),
        Step::ObservationAttributes { fields } => observation_projection(fields),
        Step::RequestedIds { trace_id, span_id } => requested_ids_step(trace_id, span_id),
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
        Step::Observation {
            span_id,
            public,
            seconds,
            datetime,
        } => observation_step(span_id, *public, *seconds, datetime.as_deref()),
        Step::StartGeneration { .. } => generation_step(step),
        Step::StartChildSpan { .. } => child_step(step),
    }
}

fn nanos(
    seconds: Option<Decimal>,
    datetime: Option<&str>,
) -> Result<serde_json::Value, Box<HelperError>> {
    if let Some(text) = datetime {
        return Ok(json!(to_unix_nanos(Some(Timestamp::DateTime(
            parse_datetime(text)?
        )))));
    }
    Ok(json!(to_unix_nanos(seconds.map(Timestamp::Seconds))))
}

fn trace_projection(fields: &TraceFields) -> serde_json::Value {
    pairs(&trace_attributes(&TraceAttributes {
        name: fields.name.as_ref(),
        user_id: fields.user_id.as_ref(),
        session_id: fields.session_id.as_ref(),
        version: fields.version.as_ref(),
        release: fields.release.as_ref(),
        tags: fields.tags.as_ref(),
        metadata: fields.metadata.as_ref(),
        public: fields.public,
        input: fields.input.as_ref(),
        output: fields.output.as_ref(),
    }))
}

fn observation_projection(
    fields: &ObservationFields,
) -> Result<serde_json::Value, Box<HelperError>> {
    let prompt = fields.prompt.as_ref().map(prompt_link);
    Ok(pairs(&observation_attributes(&ObservationAttributes {
        observation_type: observation_type(&fields.observation_type)?,
        input: fields.input.as_ref(),
        output: fields.output.as_ref(),
        metadata: fields.metadata.as_ref(),
        level: fields.level.as_ref(),
        status_message: fields.status_message.as_ref(),
        version: fields.version.as_ref(),
        model: fields.model.as_ref(),
        model_parameters: fields.model_parameters.as_ref(),
        usage_details: fields.usage_details.as_ref(),
        cost_details: fields.cost_details.as_ref(),
        completion_start_time: fields
            .completion_start_time
            .as_deref()
            .map(parse_datetime)
            .transpose()?
            .map(Timestamp::DateTime),
        prompt: prompt.as_ref(),
    })))
}

fn requested_ids_step(
    trace_id: &str,
    span_id: &str,
) -> Result<serde_json::Value, Box<HelperError>> {
    let requested_trace = parse_trace(trace_id)?;
    let requested_span = parse_span(span_id)?;
    let id_generator = RequestedIdGenerator::default();
    let requested = {
        let _scope = RequestedIds::enter(Some(requested_trace), Some(requested_span));
        (id_generator.new_trace_id(), id_generator.new_span_id())
    };
    let after = (id_generator.new_trace_id(), id_generator.new_span_id());
    Ok(json!({
        "trace_id": requested.0.to_string(),
        "span_id": requested.1.to_string(),
        "after_trace_id_length": after.0.to_string().len(),
        "after_span_id_length": after.1.to_string().len(),
        "after_trace_id_nonzero": after.0 != TraceId::INVALID,
        "after_span_id_nonzero": after.1 != SpanId::INVALID,
    }))
}

fn sampler_step(
    rate: f64,
    trace_id: &str,
    attributes: &[(String, Json)],
) -> Result<serde_json::Value, Box<HelperError>> {
    let sampled_trace = parse_trace(trace_id)?;
    let converted = key_values(attributes);
    let result = TraceIdHashSampler { rate }.should_sample(
        None,
        sampled_trace,
        "span",
        &SpanKind::Internal,
        &converted,
        &[],
    );
    let sampled = result.decision == SamplingDecision::RecordAndSample;
    Ok(json!({
        "decision": decision_name(&result.decision),
        "attributes": if sampled { pairs(&result.attributes) } else { serde_json::Value::Null },
    }))
}

const fn decision_name(decision: &SamplingDecision) -> &'static str {
    match decision {
        SamplingDecision::Drop => "drop",
        SamplingDecision::RecordOnly => "record_only",
        SamplingDecision::RecordAndSample => "record_and_sample",
    }
}

fn parent_step(
    trace_id: &str,
    parent_observation_id: Option<&str>,
    existing_trace: bool,
    fallback_span_id: &str,
) -> Result<serde_json::Value, Box<HelperError>> {
    let id_generator = FixedIdGenerator {
        span_id: parse_span(fallback_span_id)?,
    };
    let parent = parent_observation_id.map(parse_span).transpose()?;
    let context = parent_context(
        parse_trace(trace_id)?,
        parent,
        existing_trace,
        &id_generator,
    );
    let span_context = context.span().span_context().clone();
    Ok(json!({
        "valid": span_context.is_valid(),
        "trace_id": span_context.is_valid().then(|| span_context.trace_id().to_string()),
        "span_id": span_context.is_valid().then(|| span_context.span_id().to_string()),
        "is_remote": span_context.is_remote(),
        "sampled": span_context.is_sampled(),
    }))
}

#[derive(Clone, Copy, Debug)]
struct FixedIdGenerator {
    span_id: SpanId,
}

impl IdGenerator for FixedIdGenerator {
    fn new_trace_id(&self) -> TraceId {
        TraceId::INVALID
    }

    fn new_span_id(&self) -> SpanId {
        self.span_id
    }
}

fn observation_step(
    span_id: &str,
    public: Option<bool>,
    seconds: Option<Decimal>,
    datetime: Option<&str>,
) -> Result<serde_json::Value, Box<HelperError>> {
    let context = SpanContext::new(
        TraceId::INVALID,
        parse_span(span_id)?,
        TraceFlags::SAMPLED,
        false,
        TraceState::NONE,
    );
    let mut observation = LangfuseObservation::new(
        ScriptedSpan {
            context,
            parent: SpanContext::NONE,
            name: String::new(),
            start_time: None,
            attributes: Vec::new(),
            requested_trace_id: None,
            requested_span_id: None,
            end_time: None,
        },
        public,
    );
    if let Some(end_time) = timestamp(seconds, datetime)? {
        observation.end(Some(end_time));
    }
    Ok(json!({
        "id": observation.id,
        "public": observation.public,
        "end_time": observation.span.end_time.and_then(system_nanos),
    }))
}

fn generation_step(step: &Step) -> Result<serde_json::Value, Box<HelperError>> {
    let Step::StartGeneration {
        trace_id,
        observation_id,
        parent_observation_id,
        existing_trace,
        name,
        seconds,
        datetime,
        public,
        attributes,
        span_id,
    } = step
    else {
        return Err(invalid("not a generation step"));
    };
    let start = GenerationStart {
        trace_id: parse_trace(trace_id)?,
        parent_observation_id: parent_observation_id
            .as_deref()
            .map(parse_span)
            .transpose()?,
        existing_trace: *existing_trace,
        observation_id: observation_id.as_deref().map(parse_span).transpose()?,
        name: name.clone(),
        start_time: timestamp(*seconds, datetime.as_deref())?,
        public: *public,
        attributes: key_values(attributes),
    };
    Ok(generation(start, parse_span(span_id)?))
}

fn child_step(step: &Step) -> Result<serde_json::Value, Box<HelperError>> {
    let Step::StartChildSpan {
        parent_trace_id,
        parent_span_id,
        parent_public,
        name,
        seconds,
        datetime,
        attributes,
        child_span_id,
    } = step
    else {
        return Err(invalid("not a child span step"));
    };
    let parameters = child_parameters(
        ParentSpec {
            trace_id: parse_trace(parent_trace_id)?,
            span_id: parse_span(parent_span_id)?,
            public: *parent_public,
        },
        name.clone(),
        timestamp(*seconds, datetime.as_deref())?,
        key_values(attributes),
    );
    Ok(child(parameters, parse_span(child_span_id)?))
}

#[derive(Clone, Copy, Debug)]
struct ParentSpec {
    trace_id: TraceId,
    span_id: SpanId,
    public: Option<bool>,
}

struct ChildParameters {
    parent: LangfuseObservation<ScriptedSpan>,
    start: ChildSpanStart,
}

fn child_parameters(
    parent_spec: ParentSpec,
    name: String,
    start_time: Option<Timestamp>,
    attributes: Vec<KeyValue>,
) -> ChildParameters {
    let context = SpanContext::new(
        parent_spec.trace_id,
        parent_spec.span_id,
        TraceFlags::SAMPLED,
        false,
        TraceState::NONE,
    );
    ChildParameters {
        parent: LangfuseObservation::new(
            ScriptedSpan {
                context,
                parent: SpanContext::NONE,
                name: String::new(),
                start_time: None,
                attributes: Vec::new(),
                requested_trace_id: None,
                requested_span_id: None,
                end_time: None,
            },
            parent_spec.public,
        ),
        start: ChildSpanStart {
            name,
            start_time,
            attributes,
        },
    }
}

fn generation(start: GenerationStart, span_id: SpanId) -> serde_json::Value {
    let tracer = ScriptedTracer { span_id };
    let observation = start_generation(&tracer, &RandomIdGenerator::default(), start);
    let span = &observation.span;
    json!({
        "requested_trace_id": span.requested_trace_id.map(|id| id.to_string()),
        "requested_span_id": span.requested_span_id.map(|id| id.to_string()),
        "name": span.name,
        "start_time": span.start_time.and_then(system_nanos),
        "attributes": pairs(&span.attributes),
        "parent_valid": span.parent.is_valid(),
        "parent_trace_id": span.parent.is_valid().then(|| span.parent.trace_id().to_string()),
        "parent_span_id": span.parent.is_valid().then(|| span.parent.span_id().to_string()),
        "parent_is_remote": span.parent.is_remote(),
        "observation_id": observation.id,
        "public": observation.public,
    })
}

fn child(parameters: ChildParameters, span_id: SpanId) -> serde_json::Value {
    let tracer = ScriptedTracer { span_id };
    let observation = start_child_span(&tracer, &parameters.parent, parameters.start);
    let span = &observation.span;
    json!({
        "requested_trace_id": span.requested_trace_id.map(|id| id.to_string()),
        "requested_span_id": span.requested_span_id.map(|id| id.to_string()),
        "name": span.name,
        "start_time": span.start_time.and_then(system_nanos),
        "attributes": pairs(&span.attributes),
        "parent_valid": span.parent.is_valid(),
        "parent_trace_id": span.parent.is_valid().then(|| span.parent.trace_id().to_string()),
        "parent_span_id": span.parent.is_valid().then(|| span.parent.span_id().to_string()),
        "parent_is_remote": span.parent.is_remote(),
        "observation_id": observation.id,
        "public": observation.public,
    })
}

fn timestamp(
    seconds: Option<Decimal>,
    datetime: Option<&str>,
) -> Result<Option<Timestamp>, Box<HelperError>> {
    if let Some(text) = datetime {
        return Ok(Some(Timestamp::DateTime(parse_datetime(text)?)));
    }
    Ok(seconds.map(Timestamp::Seconds))
}

fn parse_datetime(text: &str) -> Result<DateTime<chrono::Utc>, Box<HelperError>> {
    DateTime::parse_from_rfc3339(text)
        .map(|parsed| parsed.with_timezone(&chrono::Utc))
        .map_err(|error| invalid(&error.to_string()))
}

fn observation_type(text: &str) -> Result<ObservationType, Box<HelperError>> {
    match text {
        "generation" => Ok(ObservationType::Generation),
        "span" => Ok(ObservationType::Span),
        _ => Err(invalid("observation_type must be generation or span")),
    }
}

fn prompt_link(fields: &PromptFields) -> PromptLink {
    PromptLink {
        name: fields.name.clone(),
        version: fields.version,
        is_fallback: fields.is_fallback,
    }
}

fn parse_trace(text: &str) -> Result<TraceId, Box<HelperError>> {
    match TraceId::from_hex(text) {
        Ok(trace_id) => Ok(trace_id),
        Err(error) => Err(invalid(&error.to_string())),
    }
}

fn parse_span(text: &str) -> Result<SpanId, Box<HelperError>> {
    match SpanId::from_hex(text) {
        Ok(span_id) => Ok(span_id),
        Err(error) => Err(invalid(&error.to_string())),
    }
}

fn key_values(attributes: &[(String, Json)]) -> Vec<KeyValue> {
    attributes
        .iter()
        .map(|(key, value)| KeyValue::new(key.clone(), value_of(value)))
        .collect()
}

fn value_of(value: &Json) -> Value {
    match value {
        Json::Bool(flag) => Value::Bool(*flag),
        Json::Number(number) => number
            .as_i64()
            .map_or_else(|| Value::from(number.to_string()), Value::I64),
        Json::Text(text) => Value::from(text.clone()),
        Json::Null | Json::List(_) | Json::Object(_) => {
            Value::from(serde_json::to_string(value).unwrap_or_else(|_error| String::new()))
        }
    }
}

fn pairs(attributes: &[KeyValue]) -> serde_json::Value {
    serde_json::Value::Array(
        attributes
            .iter()
            .map(|pair| json!([pair.key.as_str(), attribute_value(&pair.value)]))
            .collect(),
    )
}

fn attribute_value(value: &Value) -> serde_json::Value {
    match value {
        Value::Bool(flag) => json!(flag),
        Value::I64(number) => json!(number),
        Value::F64(number) => json!(number),
        Value::String(text) => json!(text.as_str()),
        Value::Array(array) => array_value(array),
        _ => serde_json::Value::Null,
    }
}

fn array_value(array: &opentelemetry::Array) -> serde_json::Value {
    match array {
        opentelemetry::Array::Bool(items) => json!(items),
        opentelemetry::Array::I64(items) => json!(items),
        opentelemetry::Array::F64(items) => json!(items),
        opentelemetry::Array::String(items) => {
            json!(items.iter().map(StringValue::as_str).collect::<Vec<_>>())
        }
        _ => serde_json::Value::Null,
    }
}

fn invalid(message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}
