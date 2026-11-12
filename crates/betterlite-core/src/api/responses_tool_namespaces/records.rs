use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_shapes::FunctionCallStatus;
use indexmap::IndexMap;

use crate::api::responses_codex_tools::records::AttributeValue;

// a dict is used as it is; an object with attributes converts through dict(), and any other truthy value converts to {}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderFields {
    Mapping(IndexMap<String, Json>),
    Attributes(AttributeValue),
}

// B004's chat tool call has no status and no provider fields, which tool output conversion reads
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionRecord {
    pub name: String,
    pub arguments: Json,
    pub status: Option<FunctionCallStatus>,
    pub provider_specific_fields: Option<ProviderFields>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCallRecord {
    pub id: String,
    pub kind: String,
    pub function: FunctionRecord,
    pub provider_specific_fields: Option<ProviderFields>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChoiceRecord {
    pub tool_calls: Vec<ToolCallRecord>,
    pub provider_specific_fields: Json,
}

// the in-memory tool-call cache belongs to B2226, so the caller stores these in order
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheWrite {
    pub key: String,
    pub value: ToolCallRecord,
}
