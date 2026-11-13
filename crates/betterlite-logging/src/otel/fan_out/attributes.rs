use crate::otel::config::destination::SpanScope;
use crate::otel::mappers::base::AttributeMap;
use crate::otel::vocabulary::constants::attribute_keys::{
    db, error, exception_event, gen_ai, litellm, litellm_error, mcp, server,
};
use opentelemetry::trace::{Event, SpanId, Status};
use opentelemetry::{Key, KeyValue, StringValue, Value};
use opentelemetry_sdk::trace::{SpanData, SpanEvents};
use std::borrow::Cow;
use std::collections::BTreeSet;
pub const LANGFUSE_TRACE_NAME: &str = "langfuse.trace.name";
const DB_KEYS: [&str; 2] = [db::SYSTEM_NAME, db::SYSTEM_LEGACY];
const DATASTORE_KEYS: [&str; 3] = [server::ADDRESS, server::PORT, db::NAMESPACE];
const OWNED_KEYS: [&str; 3] = [
    gen_ai::OPERATION_NAME,
    mcp::METHOD_NAME,
    litellm::GUARDRAIL_NAME,
];
const ERROR_KEYS: [&str; 2] = [error::MESSAGE, error::MESSAGE_LEGACY];
const UNREACHABLE: [&str; 2] = ["guardrail_failed_to_respond", "failure"];
const HEADER_PREFIXES: [&str; 2] = ["http.request.header.", "http.response.header."];
const URL_KEYS: [&str; 3] = ["http.url", "http.target", "url.full"];
const URL_QUERY: &str = "url.query";
#[derive(Clone, Copy, PartialEq, Eq)]
enum GuardrailState {
    Responded,
    Unreachable,
}

#[derive(Clone, Copy)]
struct Redaction {
    database: bool,
    owned: bool,
    guardrail: GuardrailState,
}

impl Redaction {
    fn tenant_visible(self, key: &str) -> bool {
        if HEADER_PREFIXES
            .iter()
            .any(|prefix| key.starts_with(*prefix))
            || key == litellm_error::STACK_TRACE
            || key == URL_QUERY
        {
            return false;
        }
        if self.database && DATASTORE_KEYS.contains(&key) {
            return false;
        }
        if self.guardrail == GuardrailState::Unreachable && key == litellm::GUARDRAIL_RESPONSE {
            return false;
        }
        self.owned || !ERROR_KEYS.contains(&key)
    }
}

#[must_use]
pub fn attribute_map(attributes: &[KeyValue]) -> AttributeMap {
    let mut map = AttributeMap::new();
    for pair in attributes {
        map.insert(pair.key.to_string(), pair.value.clone());
    }
    map
}

#[must_use]
pub fn is_llm_call_span(span: &SpanData) -> bool {
    let attributes = attribute_map(&span.attributes);
    let operation = attributes.contains_key(gen_ai::OPERATION_NAME);
    let mcp_call = attributes.contains_key(mcp::METHOD_NAME);
    operation && !mcp_call
}

#[must_use]
pub fn in_scope(span: &SpanData, scope: SpanScope) -> bool {
    if scope == SpanScope::Full {
        return true;
    }
    is_llm_call_span(span)
}

#[must_use]
pub fn is_excluded_database_span(attributes: &AttributeMap, excluded: &BTreeSet<String>) -> bool {
    if excluded.is_empty() {
        return false;
    }
    attributes
        .get(db::SYSTEM_NAME)
        .or_else(|| attributes.get(db::SYSTEM_LEGACY))
        .and_then(string_value)
        .is_some_and(|system| excluded.contains(system))
}

fn string_value(value: &Value) -> Option<&str> {
    match value {
        Value::String(text) => Some(text.as_str()),
        Value::Bool(_) | Value::I64(_) | Value::F64(_) | Value::Array(_) | _ => None,
    }
}

#[must_use]
pub fn tenant_view(span: &SpanData) -> SpanData {
    let attributes = attribute_map(&span.attributes);
    let redaction = Redaction {
        database: DB_KEYS.iter().any(|key| attributes.contains_key(*key)),
        owned: OWNED_KEYS.iter().any(|key| attributes.contains_key(*key)),
        guardrail: guardrail_state(&attributes),
    };
    let mut changed = !redaction.owned;
    let mut kept = Vec::with_capacity(attributes.len());
    for (key, value) in &attributes {
        if !redaction.tenant_visible(key) {
            changed = true;
            continue;
        }
        let stripped = without_query(key, value);
        changed |= stripped != *value;
        kept.push(KeyValue::new(Key::new(key.clone()), stripped));
    }
    if !changed
        && span
            .events
            .events
            .iter()
            .all(|event| !has_stack_trace(event))
    {
        return span.clone();
    }
    let mut view = span.clone();
    view.attributes = kept;
    view.dropped_attributes_count = 0;
    view.events = projected_events(span, redaction.owned);
    view.links.dropped_count = 0;
    view.status = if redaction.owned {
        span.status.clone()
    } else {
        status_without_description(&span.status)
    };
    view
}

#[must_use]
pub fn scoped(mut span: SpanData, scope: SpanScope) -> SpanData {
    if scope == SpanScope::Full {
        return span;
    }
    let mut attributes = attribute_map(&span.attributes);
    let named = attributes.contains_key(LANGFUSE_TRACE_NAME);
    if !named {
        attributes.insert(
            LANGFUSE_TRACE_NAME.to_owned(),
            Value::from(span.name.to_string()),
        );
    }
    if named && span.parent_span_id == SpanId::INVALID {
        return span;
    }
    span.attributes = attributes
        .into_iter()
        .map(|(key, value)| KeyValue::new(Key::new(key), value))
        .collect();
    span.parent_span_id = SpanId::INVALID;
    span.parent_span_is_remote = false;
    span
}

fn projected_events(span: &SpanData, owned: bool) -> SpanEvents {
    let mut events = SpanEvents::default();
    if owned {
        events.events.clone_from(&span.events.events);
        for event in &mut events.events {
            event
                .attributes
                .retain(|pair| pair.key.as_str() != exception_event::STACKTRACE);
        }
    }
    events
}

fn guardrail_state(attributes: &AttributeMap) -> GuardrailState {
    let unreachable = attributes
        .get(litellm::GUARDRAIL_STATUS)
        .and_then(string_value)
        .is_some_and(|status| UNREACHABLE.contains(&status));
    if unreachable {
        GuardrailState::Unreachable
    } else {
        GuardrailState::Responded
    }
}

fn has_stack_trace(event: &Event) -> bool {
    event
        .attributes
        .iter()
        .any(|pair| pair.key.as_str() == exception_event::STACKTRACE)
}

fn without_query(key: &str, value: &Value) -> Value {
    let Some(text) = string_value(value).filter(|_| URL_KEYS.contains(&key)) else {
        return value.clone();
    };
    let base = text.split_once('?').map_or(text, |(base, _query)| base);
    Value::String(StringValue::from(base.to_owned()))
}

const fn status_without_description(status: &Status) -> Status {
    match status {
        Status::Error { .. } => Status::Error {
            description: Cow::Borrowed(""),
        },
        Status::Ok => Status::Ok,
        Status::Unset => Status::Unset,
    }
}

#[path = "attributes/tests.rs"]
mod tests;
