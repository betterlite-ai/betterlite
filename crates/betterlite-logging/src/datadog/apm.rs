use serde::Serialize;

pub trait Span {
    fn set_tag(&self, _key: &str, _value: &str) -> bool {
        false
    }

    fn trace_id(&self) -> Option<u128> {
        None
    }

    fn span_id(&self) -> Option<u64> {
        None
    }
}

pub trait Tracer {
    fn current_span(&self) -> Option<&dyn Span> {
        None
    }

    fn current_root_span(&self) -> Option<&dyn Span> {
        None
    }

    fn active_span(&self) -> Option<&dyn Span> {
        self.current_span().or_else(|| self.current_root_span())
    }

    fn tag_active_span(&self, key: &str, value: &str) -> bool {
        let span = self.active_span();
        !key.is_empty() && span.is_some_and(|active| active.set_tag(key, value))
    }
}

// betterlite has no ddtrace client, so it always runs as LiteLLM does when ddtrace is missing
#[derive(Debug, Clone, Copy)]
pub struct NullTracer;

impl Tracer for NullTracer {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TraceContext {
    #[serde(rename = "dd.trace_id")]
    pub trace_id: String,
    #[serde(rename = "dd.span_id", skip_serializing_if = "Option::is_none")]
    pub span_id: Option<String>,
}

impl TraceContext {
    pub fn active(tracer: &dyn Tracer) -> Option<Self> {
        let span = tracer.active_span()?;
        let trace_id = span.trace_id()?.to_string();
        let span_id = span.span_id().map(|id| id.to_string());
        Some(Self { trace_id, span_id })
    }
}

pub fn tag_call_id(tracer: &dyn Tracer, call_id: Option<&str>) {
    if let Some(id) = call_id.filter(|id| !id.is_empty()) {
        tracer.tag_active_span("litellm.call_id", id);
    }
}

pub fn tag_request(
    tracer: &dyn Tracer,
    key_alias: Option<&str>,
    key_hash: Option<&str>,
    requested_model: Option<&str>,
) {
    for (key, value) in [
        ("litellm.key_alias", key_alias),
        ("litellm.key_hash", key_hash),
        ("litellm.requested_model", requested_model),
    ] {
        if let Some(text) = value.filter(|text| !text.is_empty()) {
            tracer.tag_active_span(key, text);
        }
    }
}

mod tests;
