use std::collections::BTreeMap;
use std::sync::Mutex;

use betterlite_logging::datadog::apm::{self, Span, TraceContext, Tracer};
use betterlite_logging::datadog::tagging::{self, Environment, RequestTagValue, TeamMetadata};
use serde_json::json;

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Normalize { values: Vec<String> },
    Environment(EnvironmentInput),
    Spans(SpansInput),
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EnvironmentInput {
    env: BTreeMap<String, String>,
    request_tags: Vec<RequestTagInput>,
    metadata: TeamMetadata,
}

#[derive(Debug, serde::Deserialize)]
#[serde(untagged)]
enum RequestTagInput {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    Text(String),
}

impl From<&RequestTagInput> for RequestTagValue {
    fn from(value: &RequestTagInput) -> Self {
        match value {
            RequestTagInput::Null => Self::Null,
            RequestTagInput::Bool(boolean) => Self::Bool(*boolean),
            RequestTagInput::Number(number) => Self::Number(number.to_string()),
            RequestTagInput::Text(text) => Self::Text(text.clone()),
        }
    }
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SpansInput {
    current: Option<SpanInput>,
    root: Option<SpanInput>,
    call_id: Option<String>,
    key_alias: Option<String>,
    key_hash: Option<String>,
    requested_model: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SpanInput {
    trace_id: Option<u64>,
    span_id: Option<u64>,
    set_tag_str: bool,
    set_tag: bool,
    #[serde(skip)]
    tags: Mutex<Vec<(String, String)>>,
}

impl Span for SpanInput {
    fn set_tag(&self, key: &str, value: &str) -> bool {
        let supported = self.set_tag_str || self.set_tag;
        if supported && let Ok(mut tags) = self.tags.lock() {
            tags.push((key.to_owned(), value.to_owned()));
        }
        supported
    }

    fn trace_id(&self) -> Option<u128> {
        self.trace_id.map(u128::from)
    }

    fn span_id(&self) -> Option<u64> {
        self.span_id
    }
}

struct InputTracer<'span>(Option<&'span SpanInput>, Option<&'span SpanInput>);

impl Tracer for InputTracer<'_> {
    fn current_span(&self) -> Option<&dyn Span> {
        self.0.map(|span| -> &dyn Span { span })
    }

    fn current_root_span(&self) -> Option<&dyn Span> {
        self.1.map(|span| -> &dyn Span { span })
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let output = match input {
            Input::Normalize { values } => values
                .iter()
                .map(|value| tagging::normalize_value(value))
                .collect(),
            Input::Environment(case) => {
                let environment = Environment::from_lookup(&case.env);
                json!({
                    "source": environment.source,
                    "base_url": environment.base_url,
                    "flags": [environment.use_tracer, environment.use_profiler],
                    "tags": tagging::tags(
                        &environment,
                        &case.request_tags.iter().map(RequestTagValue::from).collect::<Vec<_>>(),
                        &case.metadata,
                    ),
                })
            }
            Input::Spans(case) => {
                let tracer = InputTracer(case.current.as_ref(), case.root.as_ref());
                apm::tag_call_id(&tracer, case.call_id.as_deref());
                let (alias, hash) = (case.key_alias.as_deref(), case.key_hash.as_deref());
                apm::tag_request(&tracer, alias, hash, case.requested_model.as_deref());
                let recorded = |span: Option<&SpanInput>| Some(json!(*span?.tags.lock().ok()?));
                json!({
                    "payload": TraceContext::active(&tracer),
                    "current_tags": recorded(tracer.0),
                    "root_tags": recorded(tracer.1),
                })
            }
        };
        Ok(output)
    }
}
