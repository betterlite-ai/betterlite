use std::borrow::Cow;
use std::collections::BTreeSet;

use opentelemetry::trace::{Event, SpanId, Status};
use opentelemetry::{Key, KeyValue, StringValue, Value};
use opentelemetry_sdk::trace::{SpanData, SpanEvents, SpanLinks};

use crate::otel::config::destination::SpanScope;
use crate::otel::mappers::base::AttributeMap;
use crate::otel::vocabulary::constants::attribute_keys::{
    db, error, exception_event, gen_ai, litellm, litellm_error, mcp, server,
};

// litellm.integrations.otel.mappers.langfuse.LANGFUSE_TRACE_NAME
pub const LANGFUSE_TRACE_NAME: &str = "langfuse.trace.name";

const DB_SYSTEM_KEYS: [&str; 2] = [db::SYSTEM_NAME, db::SYSTEM_LEGACY];
// Keys on a database span that describe the proxy's own datastore: its host, its port, its schema.
const DATASTORE_ENDPOINT_KEYS: [&str; 3] = [server::ADDRESS, server::PORT, db::NAMESPACE];
const TENANT_OWNED_KEYS: [&str; 3] = [
    gen_ai::OPERATION_NAME,
    mcp::METHOD_NAME,
    litellm::GUARDRAIL_NAME,
];
const PROXY_ERROR_TEXT_KEYS: [&str; 2] = [error::MESSAGE, error::MESSAGE_LEGACY];
// The second spelling is the legacy status the request-level logger still maps.
const GUARDRAIL_UNREACHABLE_STATUSES: [&str; 2] = ["guardrail_failed_to_respond", "failure"];
// Prefixes the FastAPI instrumentor uses for headers the operator opted to capture.
const CAPTURED_HEADER_PREFIXES: [&str; 2] = ["http.request.header.", "http.response.header."];
// The instrumentor stamps the request URL with its query string, where a virtual key may ride.
const URL_KEYS: [&str; 3] = ["http.url", "http.target", "url.full"];
const URL_QUERY_KEY: &str = "url.query";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GuardrailState {
    Responded,
    Unreachable,
}

#[derive(Debug, Clone, Copy)]
struct Redaction {
    database: bool,
    owned: bool,
    guardrail: GuardrailState,
}

impl Redaction {
    // litellm.integrations.otel.plumbing.providers._tenant_visible
    fn tenant_visible(self, key: &str) -> bool {
        if CAPTURED_HEADER_PREFIXES
            .iter()
            .any(|prefix| key.starts_with(*prefix))
            || key == litellm_error::STACK_TRACE
            || key == URL_QUERY_KEY
        {
            return false;
        }
        if self.database && DATASTORE_ENDPOINT_KEYS.contains(&key) {
            return false;
        }
        if self.guardrail == GuardrailState::Unreachable && key == litellm::GUARDRAIL_RESPONSE {
            return false;
        }
        self.owned || !PROXY_ERROR_TEXT_KEYS.contains(&key)
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

// litellm.integrations.otel.plumbing.providers.is_llm_call_span
#[must_use]
pub fn is_llm_call_span(span: &SpanData) -> bool {
    let attributes = attribute_map(&span.attributes);
    let operation = attributes.contains_key(gen_ai::OPERATION_NAME);
    let mcp_call = attributes.contains_key(mcp::METHOD_NAME);
    operation && !mcp_call
}

// litellm.integrations.otel.plumbing.providers._in_scope
#[must_use]
pub fn in_scope(span: &SpanData, scope: SpanScope) -> bool {
    if scope == SpanScope::Full {
        return true;
    }
    is_llm_call_span(span)
}

// litellm.integrations.otel.plumbing.providers._is_excluded_database_span
#[must_use]
pub fn is_excluded_database_span(attributes: &AttributeMap, excluded: &BTreeSet<String>) -> bool {
    if excluded.is_empty() {
        return false;
    }
    let system = attributes
        .get(db::SYSTEM_NAME)
        .or_else(|| attributes.get(db::SYSTEM_LEGACY));
    system
        .and_then(string_value)
        .is_some_and(|name| excluded.contains(name))
}

// LiteLLM checks isinstance(value, str); every other Value variant is not a system name.
fn string_value(value: &Value) -> Option<&str> {
    match value {
        Value::String(text) => Some(text.as_str()),
        Value::Bool(_) | Value::I64(_) | Value::F64(_) | Value::Array(_) | _ => None,
    }
}

// litellm.integrations.otel.plumbing.providers._for_destination, minus the resource its exporter carries
#[must_use]
pub fn tenant_view(span: &SpanData) -> SpanData {
    let attributes = attribute_map(&span.attributes);
    let redaction = Redaction {
        database: DB_SYSTEM_KEYS
            .iter()
            .any(|key| attributes.contains_key(*key)),
        owned: TENANT_OWNED_KEYS
            .iter()
            .any(|key| attributes.contains_key(*key)),
        guardrail: guardrail_state(&attributes),
    };
    if redaction.owned && unchanged(span, &attributes, redaction) {
        return span.clone();
    }
    let mut view = span.clone();
    view.attributes = kept_attributes(&attributes, redaction);
    view.dropped_attributes_count = 0;
    view.events = projected_events(span, redaction.owned);
    view.links = zeroed_links(span);
    view.status = if redaction.owned {
        span.status.clone()
    } else {
        status_without_description(&span.status)
    };
    view
}

// litellm.integrations.otel.plumbing.providers._scoped
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

fn unchanged(span: &SpanData, attributes: &AttributeMap, redaction: Redaction) -> bool {
    let untouched_urls = |key: &str, value: &Value| {
        string_value(value).is_none_or(|text| !URL_KEYS.contains(&key) || !text.contains('?'))
    };
    attributes
        .iter()
        .all(|(key, value)| redaction.tenant_visible(key) && untouched_urls(key, value))
        && span
            .events
            .events
            .iter()
            .all(|event| !has_stack_trace(event))
}

fn kept_attributes(attributes: &AttributeMap, redaction: Redaction) -> Vec<KeyValue> {
    let mut kept = Vec::with_capacity(attributes.len());
    for (key, value) in attributes {
        if redaction.tenant_visible(key) {
            kept.push(KeyValue::new(
                Key::new(key.clone()),
                without_query(key, value),
            ));
        }
    }
    kept
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

fn zeroed_links(span: &SpanData) -> SpanLinks {
    let mut links = span.links.clone();
    links.dropped_count = 0;
    links
}

fn guardrail_state(attributes: &AttributeMap) -> GuardrailState {
    let unreachable = attributes
        .get(litellm::GUARDRAIL_STATUS)
        .and_then(string_value)
        .is_some_and(|status| GUARDRAIL_UNREACHABLE_STATUSES.contains(&status));
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
    if !URL_KEYS.contains(&key) {
        return value.clone();
    }
    let Value::String(text) = value else {
        return value.clone();
    };
    let base = text.as_str().split_once('?').map_or_else(
        || Cow::Borrowed(text.as_str()),
        |(base, _query)| Cow::Borrowed(base),
    );
    Value::String(StringValue::from(base.into_owned()))
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
