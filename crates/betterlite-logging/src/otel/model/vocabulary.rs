use crate::otel::model::{
    GuardrailSpanData, LlmCallSpanData, McpListToolsSpanData, McpToolCallSpanData,
    ProxyRequestSpanData, ServiceSpanData,
};
use std::collections::BTreeSet;
#[rustfmt::skip] macro_rules! keys {($name:ident{$($field:ident:$value:literal),*$(,)?})=>{pub struct $name;impl $name{$(pub const $field:&'static str=$value;)*}}}
#[rustfmt::skip] macro_rules! values {($name:ident{$($variant:ident:$constant:ident=$value:literal),*$(,)?})=>{#[derive(Debug,Clone,Copy,PartialEq,Eq,PartialOrd,Ord,serde::Serialize,serde::Deserialize)] pub enum $name{$($variant),*} impl $name{$(pub const $constant:Self=Self::$variant;)*#[must_use]pub const fn value(self)->&'static str{match self{$(Self::$variant=>$value),*}}}}}
#[rustfmt::skip]
macro_rules! declarations {() => {
include!("vocabulary/declarations.inc");

#[rustfmt::skip] values!(GenAIOutputType{Text:TEXT="text",Json:JSON="json",Image:IMAGE="image",Speech:SPEECH="speech"});
#[rustfmt::skip] values!(GenAIProvider{Openai:OPENAI="openai",Anthropic:ANTHROPIC="anthropic",AwsBedrock:AWS_BEDROCK="aws.bedrock",AzureAiOpenai:AZURE_AI_OPENAI="azure.ai.openai",AzureAiInference:AZURE_AI_INFERENCE="azure.ai.inference",GcpGemini:GCP_GEMINI="gcp.gemini",GcpVertexAi:GCP_VERTEX_AI="gcp.vertex_ai",Cohere:COHERE="cohere",MistralAi:MISTRAL_AI="mistral_ai",Deepseek:DEEPSEEK="deepseek",Groq:GROQ="groq",Perplexity:PERPLEXITY="perplexity",XAi:X_AI="x_ai",IbmWatsonxAi:IBM_WATSONX_AI="ibm.watsonx.ai"});
#[rustfmt::skip] values!(MCPMethod{ToolsCall:TOOLS_CALL="tools/call",ToolsList:TOOLS_LIST="tools/list",PromptsGet:PROMPTS_GET="prompts/get",PromptsList:PROMPTS_LIST="prompts/list"});
#[rustfmt::skip] values!(NetworkTransport{Tcp:TCP="tcp",Udp:UDP="udp",Quic:QUIC="quic",Unix:UNIX="unix",Pipe:PIPE="pipe"});
#[rustfmt::skip] values!(RpcSystem{JsonRpc:JSONRPC="jsonrpc"});
keys!(MCP{METHOD_NAME:"mcp.method.name",SESSION_ID:"mcp.session.id",PROTOCOL_VERSION:"mcp.protocol.version",RESOURCE_URI:"mcp.resource.uri"}); keys!(JsonRpc{SYSTEM:"rpc.system",REQUEST_ID:"jsonrpc.request.id",PROTOCOL_VERSION:"jsonrpc.protocol.version",RESPONSE_STATUS_CODE:"rpc.response.status_code"}); keys!(Network{PROTOCOL_NAME:"network.protocol.name",PROTOCOL_VERSION:"network.protocol.version",TRANSPORT:"network.transport"}); keys!(Client{ADDRESS:"client.address",PORT:"client.port"}); keys!(Error{TYPE:"error.type",MESSAGE:"error.message",MESSAGE_LEGACY:"error"}); keys!(LiteLLMError{CODE:"litellm.provider.error.code",STACK_TRACE:"litellm.provider.error.stack_trace",LLM_PROVIDER:"litellm.provider.error.llm_provider"}); keys!(ExceptionEvent{NAME:"exception",TYPE:"exception.type",MESSAGE:"exception.message",STACKTRACE:"exception.stacktrace"}); keys!(GenAIEvent{NAME_KEY:"event.name",OPERATION_EXCEPTION:"gen_ai.client.operation.exception"}); keys!(Server{ADDRESS:"server.address",PORT:"server.port"}); keys!(DB{SYSTEM_NAME:"db.system.name",SYSTEM_LEGACY:"db.system",OPERATION_NAME:"db.operation.name",NAMESPACE:"db.namespace"}); keys!(HTTP{REQUEST_METHOD:"http.request.method",ROUTE:"http.route",RESPONSE_STATUS_CODE:"http.response.status_code",URL_PATH:"url.path"});

const CHAT:&[&str]=&["completion","acompletion","completion_with_retries","responses","aresponses"];const RESPONSE:&[&str]=&["get_responses","aget_responses","delete_responses","adelete_responses","cancel_responses","acancel_responses","list_input_items","alist_input_items"];const CONTENT:&[&str]=&["image_generation","aimage_generation","ocr","aocr","speech","aspeech","transcription","atranscription"];const RETRIEVAL:&[&str]=&["vector_store_search","avector_store_search","query","aquery"];const AGENT:&[&str]=&["send_message",concat!("a","send_message"),concat!("a","send_message_streaming")];const VECTOR:&[&str]=&["vector_store_create","avector_store_create","vector_store_retrieve","avector_store_retrieve","vector_store_list","avector_store_list","vector_store_update","avector_store_update","vector_store_delete","avector_store_delete"];const VECTOR_FILE:&[&str]=&["vector_store_file_create","avector_store_file_create","vector_store_file_list","avector_store_file_list","vector_store_file_retrieve","avector_store_file_retrieve","vector_store_file_content","avector_store_file_content","vector_store_file_update","avector_store_file_update","vector_store_file_delete","avector_store_file_delete"];
#[must_use]pub fn resolve_provider(provider:Option<&str>)->String{let Some(value)=provider else{return String::new()};let mapped=match value.to_ascii_lowercase().as_str(){"openai"|"text-completion-openai"=>Some(GenAIProvider::OPENAI),"azure"=>Some(GenAIProvider::AZURE_AI_OPENAI),"azure_ai"=>Some(GenAIProvider::AZURE_AI_INFERENCE),"anthropic"=>Some(GenAIProvider::ANTHROPIC),"bedrock"|"bedrock_converse"=>Some(GenAIProvider::AWS_BEDROCK),"vertex_ai"|"vertex_ai_beta"=>Some(GenAIProvider::GCP_VERTEX_AI),"gemini"=>Some(GenAIProvider::GCP_GEMINI),"cohere"|"cohere_chat"=>Some(GenAIProvider::COHERE),"mistral"=>Some(GenAIProvider::MISTRAL_AI),"deepseek"=>Some(GenAIProvider::DEEPSEEK),"groq"=>Some(GenAIProvider::GROQ),"perplexity"=>Some(GenAIProvider::PERPLEXITY),"xai"=>Some(GenAIProvider::X_AI),"watsonx"=>Some(GenAIProvider::IBM_WATSONX_AI),_=>None};mapped.map_or_else(||value.to_owned(),|provider|provider.value().to_owned())}
#[must_use]pub fn resolve_operation(call_type:Option<&str>)->GenAIOperation{let Some(value)=call_type else{return GenAIOperation::CHAT};let normalized=value.to_ascii_lowercase();if CHAT.contains(&normalized.as_str()){return GenAIOperation::CHAT}if normalized=="text_completion"||normalized=="atext_completion"{return GenAIOperation::TEXT_COMPLETION}if normalized=="embedding"||normalized=="aembedding"{return GenAIOperation::EMBEDDINGS}if RESPONSE.contains(&normalized.as_str()){return GenAIOperation::LITELLM_RESPONSES_MANAGEMENT}if CONTENT.contains(&normalized.as_str()){return GenAIOperation::GENERATE_CONTENT}if normalized=="moderation"||normalized=="amoderation"{return GenAIOperation::LITELLM_MODERATION}if normalized=="call_mcp_tool"{return GenAIOperation::EXECUTE_TOOL}if RETRIEVAL.contains(&normalized.as_str()){return GenAIOperation::RETRIEVAL}if AGENT.contains(&normalized.as_str()){return GenAIOperation::INVOKE_AGENT}if VECTOR.contains(&normalized.as_str()){return GenAIOperation::VECTOR_MANAGEMENT}if VECTOR_FILE.contains(&normalized.as_str()){return GenAIOperation::VECTOR_FILE_MANAGEMENT}tracing::debug!(call_type=value,"unmapped OTel call type; using chat");GenAIOperation::CHAT}
#[must_use]pub fn resolve_output_type(call_type:Option<&str>)->Option<GenAIOutputType>{let value=call_type?;match value.to_ascii_lowercase().as_str(){"image_generation"|"aimage_generation"=>Some(GenAIOutputType::IMAGE),"speech"|"aspeech"=>Some(GenAIOutputType::SPEECH),"transcription"|"atranscription"|"ocr"|"aocr"=>Some(GenAIOutputType::TEXT),_=>None}}

values!(SpanRole{ProxyRequest:PROXY_REQUEST="proxy_request",LlmCall:LLM_CALL="llm_call",McpToolCall:MCP_TOOL_CALL="mcp_tool_call",McpListTools:MCP_LIST_TOOLS="mcp_list_tools",Guardrail:GUARDRAIL="guardrail",DbCall:DB_CALL="db_call",Service:SERVICE="service"}); values!(LiteLLMSpanKind{Server:SERVER="server",Client:CLIENT="client",Internal:INTERNAL="internal",Producer:PRODUCER="producer",Consumer:CONSUMER="consumer"});
#[derive(Debug,Clone,Copy,PartialEq,Eq)]pub struct SpanSpec{pub role:SpanRole,pub kind:LiteLLMSpanKind,pub parent:Option<SpanRole>}
pub static SPAN_REGISTRY:&[SpanSpec]=&[SpanSpec{role:SpanRole::PROXY_REQUEST,kind:LiteLLMSpanKind::SERVER,parent:None},SpanSpec{role:SpanRole::LLM_CALL,kind:LiteLLMSpanKind::CLIENT,parent:Some(SpanRole::PROXY_REQUEST)},SpanSpec{role:SpanRole::MCP_TOOL_CALL,kind:LiteLLMSpanKind::CLIENT,parent:Some(SpanRole::PROXY_REQUEST)},SpanSpec{role:SpanRole::MCP_LIST_TOOLS,kind:LiteLLMSpanKind::CLIENT,parent:Some(SpanRole::PROXY_REQUEST)},SpanSpec{role:SpanRole::GUARDRAIL,kind:LiteLLMSpanKind::INTERNAL,parent:Some(SpanRole::PROXY_REQUEST)},SpanSpec{role:SpanRole::DB_CALL,kind:LiteLLMSpanKind::CLIENT,parent:Some(SpanRole::PROXY_REQUEST)},SpanSpec{role:SpanRole::SERVICE,kind:LiteLLMSpanKind::INTERNAL,parent:Some(SpanRole::PROXY_REQUEST)}];
#[must_use]pub fn llm_call_span_name(data:&LlmCallSpanData)->String{format!("{} {}",data.operation.value(),data.request_model.as_deref().unwrap_or("")).trim().to_owned()}
#[must_use]pub fn mcp_tool_call_span_name(data:&McpToolCallSpanData)->String{format!("{} {}",data.method.value(),data.tool_name).trim().to_owned()}
#[must_use]pub fn mcp_list_tools_span_name(data:&McpListToolsSpanData)->String{data.method.value().to_owned()}
#[must_use]pub fn proxy_request_span_name(data:&ProxyRequestSpanData)->String{format!("{} {}",data.http_method,data.route).trim().to_owned()}
#[must_use]pub fn guardrail_span_name(data:&GuardrailSpanData)->String{format!("execute_guardrail {}",data.guardrail_name).trim().to_owned()}
#[must_use]pub fn service_span_name(data:&ServiceSpanData)->String{format!("{} {}",data.service_name,data.call_type.as_deref().unwrap_or("")).trim().to_owned()}
#[must_use]pub fn db_system(service_name:&str)->Option<&'static str>{match service_name{"redis"=>Some("redis"),"postgres"|"batch_write_to_db"=>Some("postgresql"),value if value.starts_with("redis_")=>Some("redis"),_=>None}}
#[must_use]pub fn span_role_for_service(service_name:&str)->Option<SpanRole>{match service_name{"self"|"router"|"proxy_pre_call"|"auth"=>None,value if db_system(value).is_some()=>Some(SpanRole::DB_CALL),_=>Some(SpanRole::SERVICE)}}
#[must_use]pub fn root_roles()->Vec<SpanRole>{SPAN_REGISTRY.iter().filter(|spec|spec.parent.is_none()).map(|spec|spec.role).collect()}
#[must_use]pub fn child_roles(parent:SpanRole)->Vec<SpanRole>{SPAN_REGISTRY.iter().filter(|spec|spec.parent==Some(parent)).map(|spec|spec.role).collect()}
#[derive(Debug,Clone,Copy,PartialEq,Eq)]pub enum RegistryError{MismatchedRole,UnknownParent,MissingRole}
pub fn validate_registry()->Result<(),RegistryError>{let roles=[SpanRole::PROXY_REQUEST,SpanRole::LLM_CALL,SpanRole::MCP_TOOL_CALL,SpanRole::MCP_LIST_TOOLS,SpanRole::GUARDRAIL,SpanRole::DB_CALL,SpanRole::SERVICE];for spec in SPAN_REGISTRY{if !roles.contains(&spec.role){return Err(RegistryError::MismatchedRole)}if let Some(parent)=spec.parent&&!SPAN_REGISTRY.iter().any(|candidate|candidate.role==parent){return Err(RegistryError::UnknownParent)}}if roles.iter().any(|role|!SPAN_REGISTRY.iter().any(|spec|spec.role==*role)){return Err(RegistryError::MissingRole)}Ok(())}
#[derive(Debug,Clone,Copy,PartialEq,Eq,PartialOrd,Ord)]pub enum OTELSemconvCategory{GenAiLatestExperimental}impl OTELSemconvCategory{pub const GEN_AI_LATEST_EXPERIMENTAL:Self=Self::GenAiLatestExperimental;}
#[must_use]pub fn parse_semconv_opt_in(raw:Option<&str>)->BTreeSet<OTELSemconvCategory>{let mut result=BTreeSet::new();if let Some(raw)=raw{for token in raw.split(',').map(str::trim){if token=="gen_ai_latest_experimental"{result.insert(OTELSemconvCategory::GEN_AI_LATEST_EXPERIMENTAL);}}}result}
};}
declarations!();
#[rustfmt::skip]
#[path = "vocabulary/constants/tests.rs"]
mod constants;
#[rustfmt::skip]
#[path = "vocabulary/resolvers/tests.rs"]
mod resolvers;
#[rustfmt::skip]
#[path = "vocabulary/registry/tests.rs"]
mod registry;
#[rustfmt::skip]
#[path = "vocabulary/span_names/tests.rs"]
mod span_names;
#[rustfmt::skip]
#[path = "vocabulary/semconv/tests.rs"]
mod semconv;
