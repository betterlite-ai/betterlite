use betterlite_logging::otel::model::{
    GuardrailSpanData, LlmCallSpanData, McpListToolsSpanData, McpToolCallSpanData,
    ProxyRequestSpanData, ServiceSpanData,
};
use betterlite_logging::otel::vocabulary;

#[rustfmt::skip]
macro_rules! implementation {() => {
#[derive(serde::Deserialize)]#[serde(tag="function")]pub(crate) enum Input{Provider{custom_llm_provider:Option<String>},Operation{call_type:Option<String>},OutputType{call_type:Option<String>},DbSystem{service_name:String},SpanRole{service_name:String},Registry,SpanName{kind:String,model:Option<String>,tool:Option<String>,method:Option<String>,http_method:Option<String>,route:Option<String>,guardrail:Option<String>,service:Option<String>,call_type:Option<String>},Semconv{raw:Option<String>}}
pub(crate) struct Op;
impl crate::helpers::Helper for Op{type Input=Input;fn run(input:&Self::Input)->crate::helpers::HelperResult{let output=match input{Input::Provider{custom_llm_provider}=>serde_json::json!(vocabulary::resolve_provider(custom_llm_provider.as_deref())),Input::Operation{call_type}=>serde_json::json!(vocabulary::resolve_operation(call_type.as_deref()).value()),Input::OutputType{call_type}=>serde_json::json!(vocabulary::resolve_output_type(call_type.as_deref()).map(vocabulary::GenAIOutputType::value)),Input::DbSystem{service_name}=>serde_json::json!(vocabulary::db_system(service_name)),Input::SpanRole{service_name}=>serde_json::json!(vocabulary::span_role_for_service(service_name).map(role_name)),Input::Registry=>serde_json::json!({"valid":vocabulary::validate_registry().is_ok(),"roots":vocabulary::root_roles().into_iter().map(role_name).collect::<Vec<_>>(),"children":vocabulary::child_roles(vocabulary::SpanRole::PROXY_REQUEST).into_iter().map(role_name).collect::<Vec<_>>() }),Input::SpanName{..}=>span_name(input),Input::Semconv{raw}=>serde_json::json!(vocabulary::parse_semconv_opt_in(raw.as_deref()).into_iter().map(|_|"gen_ai_latest_experimental").collect::<Vec<_>>())};Ok(output)}}
const fn role_name(role:vocabulary::SpanRole)->&'static str{match role{vocabulary::SpanRole::PROXY_REQUEST=>"proxy_request",vocabulary::SpanRole::LLM_CALL=>"llm_call",vocabulary::SpanRole::MCP_TOOL_CALL=>"mcp_tool_call",vocabulary::SpanRole::MCP_LIST_TOOLS=>"mcp_list_tools",vocabulary::SpanRole::GUARDRAIL=>"guardrail",vocabulary::SpanRole::DB_CALL=>"db_call",vocabulary::SpanRole::SERVICE=>"service"}}
fn span_name(input:&Input)->serde_json::Value{let Input::SpanName{kind,..}=input else{return serde_json::Value::Null};let name=match kind.as_str(){"llm"=>llm_name(input),"mcp_tool"=>mcp_tool_name(input),"mcp_list"=>mcp_list_name(input),"proxy"=>proxy_name(input),"guardrail"=>guardrail_name(input),"service"=>service_name(input),_=>String::new()};serde_json::json!(name)}
fn llm_name(input:&Input)->String{let Input::SpanName{model,call_type,..}=input else{return String::new()};vocabulary::llm_call_span_name(&LlmCallSpanData{operation:vocabulary::resolve_operation(call_type.as_deref()),request_model:model.clone()})}
fn mcp_tool_name(input:&Input)->String{let Input::SpanName{method:method_name,tool,..}=input else{return String::new()};let method=match method_name.as_deref().unwrap_or("tools/call"){"tools/list"=>vocabulary::MCPMethod::TOOLS_LIST,"prompts/get"=>vocabulary::MCPMethod::PROMPTS_GET,"prompts/list"=>vocabulary::MCPMethod::PROMPTS_LIST,_=>vocabulary::MCPMethod::TOOLS_CALL};vocabulary::mcp_tool_call_span_name(&McpToolCallSpanData{method,tool_name:tool.clone().unwrap_or_default()})}
fn mcp_list_name(_: &Input)->String{let data=McpListToolsSpanData{method:vocabulary::MCPMethod::TOOLS_LIST};vocabulary::mcp_list_tools_span_name(&data)}
fn proxy_name(input:&Input)->String{let Input::SpanName{http_method,route,..}=input else{return String::new()};vocabulary::proxy_request_span_name(&ProxyRequestSpanData{http_method:http_method.clone().unwrap_or_default(),route:route.clone().unwrap_or_default()})}
fn guardrail_name(input:&Input)->String{let Input::SpanName{guardrail,..}=input else{return String::new()};vocabulary::guardrail_span_name(&GuardrailSpanData{guardrail_name:guardrail.clone().unwrap_or_default()})}
fn service_name(input:&Input)->String{let Input::SpanName{service,call_type,..}=input else{return String::new()};vocabulary::service_span_name(&ServiceSpanData{service_name:service.clone().unwrap_or_default(),call_type:call_type.clone()})}
};}
implementation!();
#[path = "otel_vocabulary/tests.rs"]
mod tests;
