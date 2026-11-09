use betterlite_config::loader::EnvironmentLookup;
use opentelemetry::Value;

use super::base::{AttributeMap, AttributeMapper, AttributeTable, SpanData};
use super::utils::collect;
use crate::otel::model::payloads::llm_call::LLMCallSpanData;

// one attribute per line keeps a table readable; rustfmt stacks every closure vertically
#[rustfmt::skip]
const LLM_CALL_ATTRS: AttributeTable<LLMCallSpanData> = &[
    ("weave.display_name", |data| (!data.request_model.is_empty()).then(|| Value::from(format!("{} {}", data.operation, data.request_model)))),
    ("weave.call_id", |data| data.identity.call_id.as_deref().filter(|call_id| !call_id.is_empty()).map(|call_id| Value::from(call_id.to_owned()))),
];

#[rustfmt::skip]
const BLOB_ATTRS: AttributeTable<LLMCallSpanData> = &[
    ("weave.output", |data| {
        let text = (!data.choices_out.is_empty()).then(|| serde_json::to_string(&data.choices_out)).and_then(Result::ok);
        text.map(Value::from)
    }),
];

pub struct WeaveMapper;

impl AttributeMapper for WeaveMapper {
    fn map(&self, data: &SpanData<'_>, _environment: &dyn EnvironmentLookup) -> AttributeMap {
        match data {
            SpanData::LLMCall(call) => {
                let mut attrs = collect(LLM_CALL_ATTRS, call);
                attrs.extend(collect(BLOB_ATTRS, call));
                attrs
            }
            SpanData::MCPToolCall(_)
            | SpanData::MCPListTools(_)
            | SpanData::Guardrail(_)
            | SpanData::Service(_) => AttributeMap::new(),
        }
    }
}

#[path = "weave/tests.rs"]
mod tests;
