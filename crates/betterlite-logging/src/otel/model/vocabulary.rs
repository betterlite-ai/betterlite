#[path = "vocabulary/attribute_keys.rs"]
pub mod attribute_keys;
#[path = "vocabulary/constants.rs"]
pub mod constants;
#[path = "vocabulary/registry.rs"]
pub mod registry;
#[path = "vocabulary/resolvers.rs"]
pub mod resolvers;
#[path = "vocabulary/semconv.rs"]
pub mod semconv;
#[path = "vocabulary/span_names.rs"]
pub mod span_names;

pub type GenAIOutputType = constants::GenAIOutputType;
pub type MCPMethod = constants::MCPMethod;
pub type SpanRole = registry::SpanRole;
pub static SPAN_REGISTRY: &[registry::SpanSpec] = registry::SPAN_REGISTRY;

#[must_use]
pub fn resolve_provider(provider: Option<&str>) -> String {
    resolvers::resolve_provider(provider)
}
#[must_use]
pub fn resolve_operation(call_type: Option<&str>) -> constants::GenAIOperation {
    resolvers::resolve_operation(call_type)
}
#[must_use]
pub fn resolve_output_type(call_type: Option<&str>) -> Option<GenAIOutputType> {
    resolvers::resolve_output_type(call_type)
}
#[must_use]
pub fn db_system(service_name: &str) -> Option<&'static str> {
    resolvers::db_system(service_name)
}
#[must_use]
pub fn span_role_for_service(service_name: &str) -> Option<SpanRole> {
    resolvers::span_role_for_service(service_name)
}
pub fn validate_registry() -> Result<(), registry::RegistryError> {
    registry::validate_registry()
}
#[must_use]
pub fn root_roles() -> Vec<SpanRole> {
    registry::root_roles()
}
#[must_use]
pub fn child_roles(parent: SpanRole) -> Vec<SpanRole> {
    registry::child_roles(parent)
}
#[must_use]
pub fn llm_call_span_name(data: &LlmCallSpanData) -> String {
    span_names::llm_call_span_name(data)
}
#[must_use]
pub fn mcp_tool_call_span_name(data: &McpToolCallSpanData) -> String {
    span_names::mcp_tool_call_span_name(data)
}
#[must_use]
pub fn mcp_list_tools_span_name(data: &McpListToolsSpanData) -> String {
    span_names::mcp_list_tools_span_name(data)
}
#[must_use]
pub fn proxy_request_span_name(data: &ProxyRequestSpanData) -> String {
    span_names::proxy_request_span_name(data)
}
#[must_use]
pub fn guardrail_span_name(data: &GuardrailSpanData) -> String {
    span_names::guardrail_span_name(data)
}
#[must_use]
pub fn service_span_name(data: &ServiceSpanData) -> String {
    span_names::service_span_name(data)
}
#[must_use]
pub fn parse_semconv_opt_in(raw_value: Option<&str>) -> BTreeSet<semconv::OTELSemconvCategory> {
    semconv::parse_semconv_opt_in(raw_value)
}
use std::collections::BTreeSet;

use super::model::{
    GuardrailSpanData, LlmCallSpanData, McpListToolsSpanData, McpToolCallSpanData,
    ProxyRequestSpanData, ServiceSpanData,
};
