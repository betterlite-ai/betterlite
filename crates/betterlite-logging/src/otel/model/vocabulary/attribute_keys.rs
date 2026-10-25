pub struct GenAI;
impl GenAI {
    pub const OPERATION_NAME: &str = "gen_ai.operation.name";
    pub const PROVIDER_NAME: &str = "gen_ai.provider.name";
    pub const SYSTEM: &str = "gen_ai.system";
    pub const REQUEST_MODEL: &str = "gen_ai.request.model";
    pub const REQUEST_TEMPERATURE: &str = "gen_ai.request.temperature";
    pub const REQUEST_TOP_P: &str = "gen_ai.request.top_p";
    pub const REQUEST_TOP_K: &str = "gen_ai.request.top_k";
    pub const REQUEST_MAX_TOKENS: &str = "gen_ai.request.max_tokens";
    pub const REQUEST_FREQUENCY_PENALTY: &str = "gen_ai.request.frequency_penalty";
    pub const REQUEST_PRESENCE_PENALTY: &str = "gen_ai.request.presence_penalty";
    pub const REQUEST_STOP_SEQUENCES: &str = "gen_ai.request.stop_sequences";
    pub const REQUEST_SEED: &str = "gen_ai.request.seed";
    pub const REQUEST_CHOICE_COUNT: &str = "gen_ai.request.choice.count";
    pub const REQUEST_ENCODING_FORMATS: &str = "gen_ai.request.encoding_formats";
    pub const RESPONSE_ID: &str = "gen_ai.response.id";
    pub const RESPONSE_MODEL: &str = "gen_ai.response.model";
    pub const RESPONSE_FINISH_REASONS: &str = "gen_ai.response.finish_reasons";
    pub const RESPONSE_TIME_TO_FIRST_CHUNK: &str = "gen_ai.response.time_to_first_chunk";
    pub const USAGE_INPUT_TOKENS: &str = "gen_ai.usage.input_tokens";
    pub const USAGE_OUTPUT_TOKENS: &str = "gen_ai.usage.output_tokens";
    pub const USAGE_CACHE_CREATION_INPUT_TOKENS: &str = "gen_ai.usage.cache_creation.input_tokens";
    pub const USAGE_CACHE_READ_INPUT_TOKENS: &str = "gen_ai.usage.cache_read.input_tokens";
    pub const INPUT_MESSAGES: &str = "gen_ai.input.messages";
    pub const OUTPUT_MESSAGES: &str = "gen_ai.output.messages";
    pub const SYSTEM_INSTRUCTIONS: &str = "gen_ai.system_instructions";
    pub const OUTPUT_TYPE: &str = "gen_ai.output.type";
    pub const CONVERSATION_ID: &str = "gen_ai.conversation.id";
    pub const AGENT_ID: &str = "gen_ai.agent.id";
    pub const AGENT_NAME: &str = "gen_ai.agent.name";
    pub const TOOL_NAME: &str = "gen_ai.tool.name";
    pub const TOOL_CALL_ID: &str = "gen_ai.tool.call.id";
    pub const TOOL_CALL_ARGUMENTS: &str = "gen_ai.tool.call.arguments";
    pub const TOOL_CALL_RESULT: &str = "gen_ai.tool.call.result";
    pub const PROMPT_NAME: &str = "gen_ai.prompt.name";
}
pub struct MCP;
impl MCP {
    pub const METHOD_NAME: &str = "mcp.method.name";
    pub const SESSION_ID: &str = "mcp.session.id";
    pub const PROTOCOL_VERSION: &str = "mcp.protocol.version";
    pub const RESOURCE_URI: &str = "mcp.resource.uri";
}
pub struct JsonRpc;
impl JsonRpc {
    pub const SYSTEM: &str = "rpc.system";
    pub const REQUEST_ID: &str = "jsonrpc.request.id";
    pub const PROTOCOL_VERSION: &str = "jsonrpc.protocol.version";
    pub const RESPONSE_STATUS_CODE: &str = "rpc.response.status_code";
}
pub struct Network;
impl Network {
    pub const PROTOCOL_NAME: &str = "network.protocol.name";
    pub const PROTOCOL_VERSION: &str = "network.protocol.version";
    pub const TRANSPORT: &str = "network.transport";
}
pub struct Client;
impl Client {
    pub const ADDRESS: &str = "client.address";
    pub const PORT: &str = "client.port";
}
pub struct Error;
impl Error {
    pub const TYPE: &str = "error.type";
    pub const MESSAGE: &str = "error.message";
    pub const MESSAGE_LEGACY: &str = "error";
}
pub struct LiteLLMError;
impl LiteLLMError {
    pub const CODE: &str = "litellm.provider.error.code";
    pub const STACK_TRACE: &str = "litellm.provider.error.stack_trace";
    pub const LLM_PROVIDER: &str = "litellm.provider.error.llm_provider";
}
pub struct ExceptionEvent;
impl ExceptionEvent {
    pub const NAME: &str = "exception";
    pub const TYPE: &str = "exception.type";
    pub const MESSAGE: &str = "exception.message";
    pub const STACKTRACE: &str = "exception.stacktrace";
}
pub struct GenAIEvent;
impl GenAIEvent {
    pub const NAME_KEY: &str = "event.name";
    pub const OPERATION_EXCEPTION: &str = "gen_ai.client.operation.exception";
}
pub struct Server;
impl Server {
    pub const ADDRESS: &str = "server.address";
    pub const PORT: &str = "server.port";
}
pub struct DB;
impl DB {
    pub const SYSTEM_NAME: &str = "db.system.name";
    pub const SYSTEM_LEGACY: &str = "db.system";
    pub const OPERATION_NAME: &str = "db.operation.name";
    pub const NAMESPACE: &str = "db.namespace";
}
pub struct HTTP;
impl HTTP {
    pub const REQUEST_METHOD: &str = "http.request.method";
    pub const ROUTE: &str = "http.route";
    pub const RESPONSE_STATUS_CODE: &str = "http.response.status_code";
    pub const URL_PATH: &str = "url.path";
}
pub struct LiteLLM;
impl LiteLLM {
    pub const CALL_ID: &str = "litellm.call_id";
    pub const CALL_TYPE: &str = "litellm.call_type";
    pub const COST_PREFIX: &str = "litellm.cost.";
    pub const METADATA_PREFIX: &str = "litellm.metadata.";
    pub const TEAM_ID: &str = "litellm.team.id";
    pub const TEAM_ALIAS: &str = "litellm.team.alias";
    pub const TEAM_METADATA: &str = "litellm.team.metadata";
    pub const KEY_HASH: &str = "litellm.api_key.hash";
    pub const END_USER: &str = "litellm.end_user.id";
    pub const PROVIDER_MODEL: &str = "litellm.provider.model";
    pub const REQUEST_STREAMING: &str = "litellm.request.streaming";
    pub const REQUEST_ROUTE: &str = "litellm.request.route";
    pub const TOOLS_DECLARED: &str = "litellm.request.tools.declared";
    pub const GUARDRAIL_NAME: &str = "litellm.guardrail.name";
    pub const GUARDRAIL_MODE: &str = "litellm.guardrail.mode";
    pub const GUARDRAIL_STATUS: &str = "litellm.guardrail.status";
    pub const GUARDRAIL_PROVIDER: &str = "litellm.guardrail.provider";
    pub const GUARDRAIL_ACTION: &str = "litellm.guardrail.action";
    pub const GUARDRAIL_RESPONSE: &str = "litellm.guardrail.response";
    pub const GUARDRAIL_VIOLATION_CATEGORIES: &str = "litellm.guardrail.violation_categories";
    pub const GUARDRAIL_CONFIDENCE_SCORE: &str = "litellm.guardrail.confidence_score";
    pub const GUARDRAIL_RISK_SCORE: &str = "litellm.guardrail.risk_score";
    pub const GUARDRAIL_MASKED_ENTITY_COUNT: &str = "litellm.guardrail.masked_entity_count";
    pub const GUARDRAIL_DURATION: &str = "litellm.guardrail.duration";
    pub const GUARDRAIL_ID: &str = "litellm.guardrail.id";
    pub const GUARDRAIL_POLICY_TEMPLATE: &str = "litellm.guardrail.policy_template";
    pub const GUARDRAIL_DETECTION_METHOD: &str = "litellm.guardrail.detection_method";
    pub const GUARDRAIL_USAGE: &str = "litellm.guardrail.usage";
    pub const GUARDRAIL_COST: &str = "litellm.cost.guardrail";
    pub const GUARDRAIL_COST_IN_SPEND: &str = "litellm.guardrail.cost_in_spend";
    pub const SERVICE_NAME: &str = "litellm.service.name";
    pub const SERVICE_CALL_TYPE: &str = "litellm.service.call_type";
    pub const SERVICE_CALLER: &str = "litellm.service.caller";
    pub const PREPROCESSING_MS: &str = "litellm.preprocessing.duration_ms";
    pub const MCP_SERVER_NAME: &str = "litellm.mcp.server.name";
}
pub struct Metric;
impl Metric {
    pub const TOKEN_USAGE: &str = "gen_ai.client.token.usage";
    pub const OPERATION_DURATION: &str = "gen_ai.client.operation.duration";
    pub const TOKEN_COST: &str = "gen_ai.usage.cost";
    pub const TIME_TO_FIRST_TOKEN: &str = "gen_ai.server.time_to_first_token";
    pub const TIME_PER_OUTPUT_TOKEN: &str = "gen_ai.server.time_per_output_token";
    pub const RESPONSE_DURATION: &str = "gen_ai.client.response.duration";
}
