use betterlite_config::loader::EnvironmentLookup;
use opentelemetry::Value;
use std::collections::BTreeMap;

use crate::otel::model::payloads::guardrail::GuardrailSpanData;
use crate::otel::model::payloads::llm_call::LLMCallSpanData;
use crate::otel::model::payloads::mcp_call::{MCPListToolsSpanData, MCPToolCallSpanData};
use crate::otel::model::payloads::service::ServiceSpanData;

pub type AttributeMap = BTreeMap<String, Value>;

pub type Extractor<T> = fn(&T) -> Option<Value>;

pub type AttributeTable<T> = &'static [(&'static str, Extractor<T>)];

pub enum SpanData<'data> {
    LLMCall(&'data LLMCallSpanData),
    MCPToolCall(&'data MCPToolCallSpanData),
    MCPListTools(&'data MCPListToolsSpanData),
    Guardrail(&'data GuardrailSpanData),
    Service(&'data ServiceSpanData),
}

pub trait AttributeMapper {
    // the environment reaches the datastore attributes on every call, as LiteLLM reads it per span
    fn map(&self, data: &SpanData<'_>, environment: &dyn EnvironmentLookup) -> AttributeMap;
}

// the test modules mirror litellm.integrations.otel.mappers so the task's filters find them
#[path = "base/otel_mappers/mod.rs"]
mod otel_mappers;
