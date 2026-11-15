use crate::guardrails::form_tables_a::model::{Extra, Field, Initial, Model, Shape};

pub(crate) const TABLES: &[(&str, Model)] = &[
    ("decision_model", DECISION_MODEL),
    ("crowdstrike_aidr", CROWDSTRIKE_AIDR),
    ("panw_prisma_airs", PANW_PRISMA_AIRS),
    ("noma", NOMA),
    ("noma_v2", NOMA_V2),
    ("conduct", CONDUCT),
    ("typesafe", TYPESAFE),
    ("dynamoai", DYNAMOAI),
    ("headroom", HEADROOM),
    ("promptguard", PROMPTGUARD),
    ("qualifire", QUALIFIRE),
];

#[rustfmt::skip]
const DECISION_MODEL: Model = Model { name: Some("Decision Model"), fields: &[
    Field { name: "checks", description: "Predicates the decision model answers about each request or response. A check flagged at or above its threshold blocks (action 'block') or is recorded (action 'log').", required: true, shape: Shape::Other, default: Initial::Null, extra: Extra::NONE },
    Field { name: "decision_model", description: "Decisions-API model that scores each check (e.g. a Jev deployment on this proxy)", required: true, shape: Shape::Text, default: Initial::Null, extra: Extra::NONE },
    Field { name: "max_concurrent_decision_calls", description: "Maximum decisions calls in flight at once on this guardrail in each proxy worker. Calls beyond it wait for a free slot", required: false, shape: Shape::Number, default: Initial::Int(8), extra: Extra::NONE },
    Field { name: "max_input_chars", description: "Character budget for each text sent to the decision model. Texts longer than this are split into overlapping chunks that are each checked, so long inputs are fully screened", required: false, shape: Shape::Number, default: Initial::Int(24_000), extra: Extra::NONE },
    Field { name: "optional_params", description: "Optional parameters for the guardrail", required: false, shape: Shape::Other, default: Initial::None, extra: Extra::NONE },
    Field { name: "streaming_end_of_stream_only", description: "For streamed responses, skip the scan every streaming_sampling_rate chunks and score the assembled response once at end of stream", required: false, shape: Shape::Boolean, default: Initial::Bool(false), extra: Extra::NONE },
    Field { name: "streaming_sampling_rate", description: "For streamed responses, score the accumulated response every Nth chunk. Each scan is a decisions call", required: false, shape: Shape::Number, default: Initial::Int(5), extra: Extra::NONE },
] };

#[rustfmt::skip]
const CROWDSTRIKE_AIDR: Model = Model { name: Some("CrowdStrike AIDR Guardrail"), fields: &[
    Field { name: "api_base", description: "The CrowdStrike AIDR API base URL. Reads from CS_AIDR_BASE_URL env var if None.", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
    Field { name: "api_key", description: "The CrowdStrike AIDR API key. Reads from CS_AIDR_TOKEN env var if None.", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
    Field { name: "fail_on_error", description: "When False, errors calling the AIDR guard API (connection failures, timeouts, 4xx/5xx responses, malformed reply bodies) fail open and the request proceeds unmodified. A blocked verdict delivered on a success response still blocks, and a transformed response that cannot be parsed fails closed so delivered redactions are never dropped.", required: false, shape: Shape::Boolean, default: Initial::Bool(true), extra: Extra::NONE },
    Field { name: "optional_params", description: "Optional parameters for the guardrail", required: false, shape: Shape::Nested(&CROWDSTRIKE_OPTIONS), default: Initial::None, extra: Extra::NONE },
] };

#[rustfmt::skip]
const CROWDSTRIKE_OPTIONS: Model = Model { name: None, fields: &[
    Field { name: "streaming_buffer_release_on_scan", description: "When buffering, release withheld chunks after each passing scan. Defaults to False when unset.", required: false, shape: Shape::Boolean, default: Initial::None, extra: Extra::NONE },
    Field { name: "streaming_buffer_until_moderated", description: "When True, withhold streamed chunks until moderation passes. Defaults to False when unset.", required: false, shape: Shape::Boolean, default: Initial::None, extra: Extra::NONE },
    Field { name: "streaming_end_of_stream_only", description: "If False (default when unset), post_call scans the accumulated streamed response every streaming_sampling_rate chunks and an in-flight block stops the stream. If True, the guard runs once over the assembled response at end of stream, so flagged content may already have reached the client.", required: false, shape: Shape::Boolean, default: Initial::None, extra: Extra::NONE },
    Field { name: "streaming_sampling_rate", description: "When streaming_end_of_stream_only is False, scan the accumulated streamed response every Nth chunk. Defaults to 5 when unset.", required: false, shape: Shape::Number, default: Initial::None, extra: Extra::NONE },
] };

#[rustfmt::skip]
const PANW_PRISMA_AIRS: Model = Model { name: Some("PANW Prisma AIRS"), fields: &[
    Field { name: "api_base", description: "The API base for the PANW Prisma AIRS guardrail. Defaults to https://service.api.aisecurity.paloaltonetworks.com. If not provided, the `PANW_PRISMA_AIRS_API_BASE` environment variable is checked.", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
    Field { name: "api_key", description: "The API key for the PANW Prisma AIRS guardrail. If not provided, the `PANW_PRISMA_AIRS_API_KEY` environment variable is checked.", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
    Field { name: "app_name", description: "Application name for tracking this LiteLLM instance in Prisma AIRS analytics and dashboards. Defaults to 'LiteLLM' if not specified.", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
    Field { name: "experimental_use_latest_role_message_only", description: "Scan only the latest user/developer message on the request side instead of the full conversation history. Set true to enable for every request shape (chat completions, Anthropic /v1/messages, /v1/responses); set false to always scan all user/system/developer messages. When unset: latest-only for Anthropic /v1/messages, full history otherwise. Latest-only trusts caller-supplied history: earlier turns are not rescanned, so enable it only where each turn was scanned when it was the latest message or history is server-controlled.", required: false, shape: Shape::Boolean, default: Initial::None, extra: Extra::NONE },
    Field { name: "fallback_on_error", description: "Action when PANW API is unavailable (timeout, rate limit, network error): 'block' (default, maximum security) rejects requests; 'allow' (high availability) proceeds without scanning. Authentication and configuration errors always block.", required: false, shape: Shape::Choice(&["block", "allow"]), default: Initial::Text("block"), extra: Extra::NONE },
    Field { name: "mask_on_block", description: "Backwards compatible flag that enables both request and response masking. When True, enables both mask_request_content and mask_response_content.", required: false, shape: Shape::Boolean, default: Initial::Bool(false), extra: Extra::NONE },
    Field { name: "mask_request_content", description: "Apply masking to prompts that would be blocked. When True, masked content is sent to the LLM instead of blocking the request.", required: false, shape: Shape::Boolean, default: Initial::Bool(false), extra: Extra::NONE },
    Field { name: "mask_response_content", description: "Apply masking to responses that would be blocked. When True, masked content is returned to the user instead of blocking the response.", required: false, shape: Shape::Boolean, default: Initial::Bool(false), extra: Extra::NONE },
    Field { name: "profile_name", description: "PANW Prisma AIRS security profile name configured in Strata Cloud Manager. Optional if API key has a linked profile.", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
    Field { name: "timeout", description: "PANW API call timeout in seconds (1-60).", required: false, shape: Shape::Number, default: Initial::Float(10.0), extra: Extra::NONE },
] };

#[rustfmt::skip]
const NOMA: Model = Model { name: Some("Noma Security"), fields: &[
    Field { name: "api_base", description: "The Noma API base URL. Defaults to https://api.noma.security. Also checks if the NOMA_API_KEY env var is set.", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
    Field { name: "api_key", description: "The Noma API key. Reads from NOMA_API_KEY env var if None.", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
    Field { name: "application_id", description: "The Noma Application ID. Reads from NOMA_APPLICATION_ID env var if None.", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
    Field { name: "use_v2", description: "If True and guardrail='noma', route to the new Noma v2 implementation.", required: false, shape: Shape::Boolean, default: Initial::Bool(false), extra: Extra::NONE },
] };

#[rustfmt::skip]
const NOMA_V2: Model = Model { name: Some("Noma Security v2"), fields: &[
    Field { name: "api_base", description: "The Noma API base URL. Defaults to https://api.noma.security.", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
    Field { name: "api_key", description: "The Noma API key. Reads from NOMA_API_KEY env var if None.", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
    Field { name: "application_id", description: "The Noma Application ID. Reads from NOMA_APPLICATION_ID env var if None.", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
    Field { name: "block_failures", description: "When true, fail closed on Noma API errors.", required: false, shape: Shape::Boolean, default: Initial::None, extra: Extra::NONE },
    Field { name: "gateway_name", description: "Gateway name, used as the gateway_host label on Noma scans. Falls back to NOMA_GATEWAY_NAME.", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
    Field { name: "monitor_mode", description: "When true, run guardrail checks in monitor mode.", required: false, shape: Shape::Boolean, default: Initial::None, extra: Extra::NONE },
] };

#[rustfmt::skip]
const CONDUCT: Model = Model { name: Some("Conduct Guard"), fields: &[
    Field { name: "api_base", description: "Conduct API base URL. The MCP endpoint is derived as <api_base>/mcp.", required: false, shape: Shape::Text, default: Initial::Text("https://api.conductai.ai"), extra: Extra::NONE },
    Field { name: "api_key", description: "Conduct agent token. Env: CONDUCT_AGENT_TOKEN.", required: true, shape: Shape::Text, default: Initial::Null, extra: Extra::NONE },
    Field { name: "optional_params", description: "Optional parameters for the guardrail", required: false, shape: Shape::Nested(&CONDUCT_OPTIONS), default: Initial::None, extra: Extra::NONE },
] };

#[rustfmt::skip]
const CONDUCT_OPTIONS: Model = Model { name: None, fields: &[
    Field { name: "timeout", description: "Timeout in seconds for the Conduct check.", required: false, shape: Shape::Number, default: Initial::Float(8.0), extra: Extra::NONE },
    Field { name: "tool_name", description: "Conduct tool name the prompt is evaluated under. Match the tool your rules target.", required: false, shape: Shape::Text, default: Initial::Text("llm_call"), extra: Extra::NONE },
    Field { name: "unreachable_fallback", description: "Behavior when Conduct is unreachable, times out, or rejects the token.", required: false, shape: Shape::Choice(&["fail_open", "fail_closed"]), default: Initial::Text("fail_closed"), extra: Extra::NONE },
    Field { name: "workspace_id", description: "Conduct workspace id, sent as the X-Workspace-Id header. Env: CONDUCT_WORKSPACE_ID.", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
] };

#[rustfmt::skip]
const TYPESAFE: Model = Model { name: Some("TypeSafe (Jev) Compaction"), fields: &[
    Field { name: "api_base", description: "Base URL of the TypeSafe API. Falls back to the TYPESAFE_API_BASE env var, then https://api.typesafe.ai.", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
    Field { name: "api_key", description: "TypeSafe API key, sent as a Bearer token. Falls back to the TYPESAFE_API_KEY env var.", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
    Field { name: "model", description: "TypeSafe evaluation model (not the LLM). Defaults to 'jev-latest'.", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
    Field { name: "optional_params", description: "Optional parameters for the guardrail", required: false, shape: Shape::Nested(&TYPESAFE_OPTIONS), default: Initial::None, extra: Extra::NONE },
    Field { name: "unreachable_fallback", description: "Behavior when the TypeSafe evaluation service is unreachable or errors. 'fail_open' (default) forwards the request uncompacted. 'fail_closed' raises an error instead.", required: false, shape: Shape::Choice(&["fail_closed", "fail_open"]), default: Initial::Text("fail_open"), extra: Extra::NONE },
] };

#[rustfmt::skip]
const TYPESAFE_OPTIONS: Model = Model { name: None, fields: &[
    Field { name: "max_result_chars_in_state", description: "Tool result text is truncated to this many characters when sent to the Jev evaluator, keeping the head and tail. Defaults to 4000.", required: false, shape: Shape::Number, default: Initial::None, extra: Extra::NONE },
    Field { name: "min_chars_to_evaluate", description: "Skip tool exchanges whose combined tool-result text is shorter than this many characters. Defaults to 200.", required: false, shape: Shape::Number, default: Initial::None, extra: Extra::NONE },
    Field { name: "relevance_threshold", description: "Relevance cutoff in [0, 1]. A completed tool exchange is dropped when Jev scores the probability that it is still needed below this value. Defaults to 0.2.", required: false, shape: Shape::Number, default: Initial::None, extra: Extra::NONE },
] };

#[rustfmt::skip]
const DYNAMOAI: Model = Model { name: Some("DynamoAI Guardrails"), fields: &[
    Field { name: "api_base", description: "Base URL for DynamoAI API. If not provided, the `DYNAMOAI_API_BASE` environment variable is checked, defaults to https://api.dynamo.ai", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
    Field { name: "api_key", description: "API key for DynamoAI Guardrails. If not provided, the `DYNAMOAI_API_KEY` environment variable is checked.", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
    Field { name: "guardrail_name", description: "Name of the guardrail for identification in logs and traces.", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
    Field { name: "model_id", description: "Model ID for tracking/logging purposes. If not provided, the `DYNAMOAI_MODEL_ID` environment variable is checked.", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
    Field { name: "policy_ids", description: "List of DynamoAI policy IDs to apply. If not provided, the `DYNAMOAI_POLICY_IDS` environment variable is checked (comma-separated).", required: false, shape: Shape::List(&[]), default: Initial::None, extra: Extra::NONE },
] };

#[rustfmt::skip]
const HEADROOM: Model = Model { name: Some("Headroom"), fields: &[
    Field { name: "api_base", description: "Base URL for the headroom compression service (e.g. https://api.headroom.ai). Falls back to HEADROOM_API_BASE env var.", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
    Field { name: "api_key", description: "API key for the headroom compression service. Falls back to HEADROOM_API_KEY env var.", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
    Field { name: "ccr_retrieval", description: "Inject the Headroom retrieval tool for hashes declared by the compression service.", required: false, shape: Shape::Boolean, default: Initial::Bool(true), extra: Extra::NONE },
    Field { name: "model", description: "Model name forwarded to the headroom /v1/compress endpoint.", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
    Field { name: "optional_params", description: "Optional parameters for the guardrail", required: false, shape: Shape::Other, default: Initial::None, extra: Extra::NONE },
    Field { name: "unreachable_fallback", description: "Behavior when the headroom compression service is unreachable or errors. 'fail_closed' raises an error (default). 'fail_open' logs a critical error and forwards the request uncompressed instead of blocking it.", required: false, shape: Shape::Choice(&["fail_closed", "fail_open"]), default: Initial::Text("fail_closed"), extra: Extra::NONE },
] };

#[rustfmt::skip]
const PROMPTGUARD: Model = Model { name: Some("PromptGuard"), fields: &[
    Field { name: "api_base", description: "PromptGuard API base URL. Defaults to https://api.promptguard.co. Falls back to PROMPTGUARD_API_BASE env var.", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
    Field { name: "api_key", description: "API key for PromptGuard authentication. If not provided, the PROMPTGUARD_API_KEY environment variable is used.", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
    Field { name: "block_on_error", description: "Whether to block the request when the PromptGuard API is unreachable. Defaults to true (fail-closed). Set to false for fail-open behaviour.", required: false, shape: Shape::Boolean, default: Initial::None, extra: Extra::NONE },
] };

#[rustfmt::skip]
const QUALIFIRE: Model = Model { name: Some("Qualifire"), fields: &[
    Field { name: "api_base", description: "The API base URL for Qualifire. If not provided, the `QUALIFIRE_BASE_URL` environment variable is checked.", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
    Field { name: "api_key", description: "The API key for Qualifire. If not provided, the `QUALIFIRE_API_KEY` environment variable is checked.", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
    Field { name: "assertions", description: "Custom assertions to validate against the output. Each assertion is a string describing a condition.", required: false, shape: Shape::List(&[]), default: Initial::None, extra: Extra::NONE },
    Field { name: "content_moderation_check", description: "Enable content moderation to check for harmful content (harassment, hate speech, etc.).", required: false, shape: Shape::Boolean, default: Initial::None, extra: Extra::NONE },
    Field { name: "evaluation_id", description: "Pre-configured evaluation ID from Qualifire dashboard. When provided, uses invoke_evaluation() instead of evaluate().", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
    Field { name: "grounding_check", description: "Enable grounding verification to ensure output is grounded in provided context.", required: false, shape: Shape::Boolean, default: Initial::None, extra: Extra::NONE },
    Field { name: "hallucinations_check", description: "Enable hallucination detection to detect factual inaccuracies.", required: false, shape: Shape::Boolean, default: Initial::None, extra: Extra::NONE },
    Field { name: "on_flagged", description: "Action to take when content is flagged. 'block' raises an exception, 'monitor' logs but allows the request.", required: false, shape: Shape::Choice(&["block", "monitor"]), default: Initial::Text("block"), extra: Extra::NONE },
    Field { name: "pii_check", description: "Enable PII (Personally Identifiable Information) detection.", required: false, shape: Shape::Boolean, default: Initial::None, extra: Extra::NONE },
    Field { name: "prompt_injections", description: "Enable prompt injection detection. Default check if no evaluation_id and no other checks are specified.", required: false, shape: Shape::Boolean, default: Initial::None, extra: Extra::NONE },
    Field { name: "tool_selection_quality_check", description: "Enable tool selection quality check to evaluate quality of tool/function calls.", required: false, shape: Shape::Boolean, default: Initial::None, extra: Extra::NONE },
] };
