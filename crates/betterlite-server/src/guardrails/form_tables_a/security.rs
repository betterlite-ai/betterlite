use serde_json::{Value, json};

use crate::guardrails::provider_forms::spec::{FormExtra, FormField, FormModel, FormShape};

#[must_use]
pub(crate) fn tables() -> Vec<(&'static str, FormModel)> {
    vec![
        ("decision_model", decision_model()),
        ("crowdstrike_aidr", crowdstrike_aidr()),
        ("panw_prisma_airs", panw_prisma_airs()),
        ("noma", noma()),
        ("noma_v2", noma_v2()),
        ("conduct", conduct()),
        ("typesafe", typesafe()),
        ("dynamoai", dynamoai()),
        ("headroom", headroom()),
        ("promptguard", promptguard()),
        ("qualifire", qualifire()),
    ]
}

#[rustfmt::skip]
fn decision_model() -> FormModel {
    FormModel { ui_friendly_name: Some("Decision Model"), fields: vec![
        FormField { name: "checks", description: Some("Predicates the decision model answers about each request or response. A check flagged at or above its threshold blocks (action 'block') or is recorded (action 'log')."), required: true, shape: FormShape::Other, default: Some(Value::Null), extra: FormExtra::NONE },
        FormField { name: "decision_model", description: Some("Decisions-API model that scores each check (e.g. a Jev deployment on this proxy)"), required: true, shape: FormShape::Text, default: Some(Value::Null), extra: FormExtra::NONE },
        FormField { name: "max_concurrent_decision_calls", description: Some("Maximum decisions calls in flight at once on this guardrail in each proxy worker. Calls beyond it wait for a free slot"), required: false, shape: FormShape::Number, default: Some(json!(8_i64)), extra: FormExtra::NONE },
        FormField { name: "max_input_chars", description: Some("Character budget for each text sent to the decision model. Texts longer than this are split into overlapping chunks that are each checked, so long inputs are fully screened"), required: false, shape: FormShape::Number, default: Some(json!(24_000_i64)), extra: FormExtra::NONE },
        FormField { name: "optional_params", description: Some("Optional parameters for the guardrail"), required: false, shape: FormShape::Other, default: None, extra: FormExtra::NONE },
        FormField { name: "streaming_end_of_stream_only", description: Some("For streamed responses, skip the scan every streaming_sampling_rate chunks and score the assembled response once at end of stream"), required: false, shape: FormShape::Boolean, default: Some(json!(false)), extra: FormExtra::NONE },
        FormField { name: "streaming_sampling_rate", description: Some("For streamed responses, score the accumulated response every Nth chunk. Each scan is a decisions call"), required: false, shape: FormShape::Number, default: Some(json!(5_i64)), extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
fn crowdstrike_aidr() -> FormModel {
    FormModel { ui_friendly_name: Some("CrowdStrike AIDR Guardrail"), fields: vec![
        FormField { name: "api_base", description: Some("The CrowdStrike AIDR API base URL. Reads from CS_AIDR_BASE_URL env var if None."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "api_key", description: Some("The CrowdStrike AIDR API key. Reads from CS_AIDR_TOKEN env var if None."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "fail_on_error", description: Some("When False, errors calling the AIDR guard API (connection failures, timeouts, 4xx/5xx responses, malformed reply bodies) fail open and the request proceeds unmodified. A blocked verdict delivered on a success response still blocks, and a transformed response that cannot be parsed fails closed so delivered redactions are never dropped."), required: false, shape: FormShape::Boolean, default: Some(json!(true)), extra: FormExtra::NONE },
        FormField { name: "optional_params", description: Some("Optional parameters for the guardrail"), required: false, shape: FormShape::Nested(Box::new(crowdstrike_aidr_optional_params_model())), default: None, extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
fn crowdstrike_aidr_optional_params_model() -> FormModel {
    FormModel { ui_friendly_name: None, fields: vec![
        FormField { name: "streaming_buffer_release_on_scan", description: Some("When buffering, release withheld chunks after each passing scan. Defaults to False when unset."), required: false, shape: FormShape::Boolean, default: None, extra: FormExtra::NONE },
        FormField { name: "streaming_buffer_until_moderated", description: Some("When True, withhold streamed chunks until moderation passes. Defaults to False when unset."), required: false, shape: FormShape::Boolean, default: None, extra: FormExtra::NONE },
        FormField { name: "streaming_end_of_stream_only", description: Some("If False (default when unset), post_call scans the accumulated streamed response every streaming_sampling_rate chunks and an in-flight block stops the stream. If True, the guard runs once over the assembled response at end of stream, so flagged content may already have reached the client."), required: false, shape: FormShape::Boolean, default: None, extra: FormExtra::NONE },
        FormField { name: "streaming_sampling_rate", description: Some("When streaming_end_of_stream_only is False, scan the accumulated streamed response every Nth chunk. Defaults to 5 when unset."), required: false, shape: FormShape::Number, default: None, extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
fn panw_prisma_airs() -> FormModel {
    FormModel { ui_friendly_name: Some("PANW Prisma AIRS"), fields: vec![
        FormField { name: "api_base", description: Some("The API base for the PANW Prisma AIRS guardrail. Defaults to https://service.api.aisecurity.paloaltonetworks.com. If not provided, the `PANW_PRISMA_AIRS_API_BASE` environment variable is checked."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "api_key", description: Some("The API key for the PANW Prisma AIRS guardrail. If not provided, the `PANW_PRISMA_AIRS_API_KEY` environment variable is checked."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "app_name", description: Some("Application name for tracking this LiteLLM instance in Prisma AIRS analytics and dashboards. Defaults to 'LiteLLM' if not specified."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "experimental_use_latest_role_message_only", description: Some("Scan only the latest user/developer message on the request side instead of the full conversation history. Set true to enable for every request shape (chat completions, Anthropic /v1/messages, /v1/responses); set false to always scan all user/system/developer messages. When unset: latest-only for Anthropic /v1/messages, full history otherwise. Latest-only trusts caller-supplied history: earlier turns are not rescanned, so enable it only where each turn was scanned when it was the latest message or history is server-controlled."), required: false, shape: FormShape::Boolean, default: None, extra: FormExtra::NONE },
        FormField { name: "fallback_on_error", description: Some("Action when PANW API is unavailable (timeout, rate limit, network error): 'block' (default, maximum security) rejects requests; 'allow' (high availability) proceeds without scanning. Authentication and configuration errors always block."), required: false, shape: FormShape::Choice(vec![json!("block"), json!("allow")]), default: Some(json!("block")), extra: FormExtra::NONE },
        FormField { name: "mask_on_block", description: Some("Backwards compatible flag that enables both request and response masking. When True, enables both mask_request_content and mask_response_content."), required: false, shape: FormShape::Boolean, default: Some(json!(false)), extra: FormExtra::NONE },
        FormField { name: "mask_request_content", description: Some("Apply masking to prompts that would be blocked. When True, masked content is sent to the LLM instead of blocking the request."), required: false, shape: FormShape::Boolean, default: Some(json!(false)), extra: FormExtra::NONE },
        FormField { name: "mask_response_content", description: Some("Apply masking to responses that would be blocked. When True, masked content is returned to the user instead of blocking the response."), required: false, shape: FormShape::Boolean, default: Some(json!(false)), extra: FormExtra::NONE },
        FormField { name: "profile_name", description: Some("PANW Prisma AIRS security profile name configured in Strata Cloud Manager. Optional if API key has a linked profile."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "timeout", description: Some("PANW API call timeout in seconds (1-60)."), required: false, shape: FormShape::Number, default: Some(json!(10.0_f64)), extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
fn noma() -> FormModel {
    FormModel { ui_friendly_name: Some("Noma Security"), fields: vec![
        FormField { name: "api_base", description: Some("The Noma API base URL. Defaults to https://api.noma.security. Also checks if the NOMA_API_KEY env var is set."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "api_key", description: Some("The Noma API key. Reads from NOMA_API_KEY env var if None."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "application_id", description: Some("The Noma Application ID. Reads from NOMA_APPLICATION_ID env var if None."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "use_v2", description: Some("If True and guardrail='noma', route to the new Noma v2 implementation."), required: false, shape: FormShape::Boolean, default: Some(json!(false)), extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
fn noma_v2() -> FormModel {
    FormModel { ui_friendly_name: Some("Noma Security v2"), fields: vec![
        FormField { name: "api_base", description: Some("The Noma API base URL. Defaults to https://api.noma.security."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "api_key", description: Some("The Noma API key. Reads from NOMA_API_KEY env var if None."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "application_id", description: Some("The Noma Application ID. Reads from NOMA_APPLICATION_ID env var if None."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "block_failures", description: Some("When true, fail closed on Noma API errors."), required: false, shape: FormShape::Boolean, default: None, extra: FormExtra::NONE },
        FormField { name: "gateway_name", description: Some("Gateway name, used as the gateway_host label on Noma scans. Falls back to NOMA_GATEWAY_NAME."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "monitor_mode", description: Some("When true, run guardrail checks in monitor mode."), required: false, shape: FormShape::Boolean, default: None, extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
fn conduct() -> FormModel {
    FormModel { ui_friendly_name: Some("Conduct Guard"), fields: vec![
        FormField { name: "api_base", description: Some("Conduct API base URL. The MCP endpoint is derived as <api_base>/mcp."), required: false, shape: FormShape::Text, default: Some(json!("https://api.conductai.ai")), extra: FormExtra::NONE },
        FormField { name: "api_key", description: Some("Conduct agent token. Env: CONDUCT_AGENT_TOKEN."), required: true, shape: FormShape::Text, default: Some(Value::Null), extra: FormExtra::NONE },
        FormField { name: "optional_params", description: Some("Optional parameters for the guardrail"), required: false, shape: FormShape::Nested(Box::new(conduct_optional_params_model())), default: None, extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
fn conduct_optional_params_model() -> FormModel {
    FormModel { ui_friendly_name: None, fields: vec![
        FormField { name: "timeout", description: Some("Timeout in seconds for the Conduct check."), required: false, shape: FormShape::Number, default: Some(json!(8.0_f64)), extra: FormExtra::NONE },
        FormField { name: "tool_name", description: Some("Conduct tool name the prompt is evaluated under. Match the tool your rules target."), required: false, shape: FormShape::Text, default: Some(json!("llm_call")), extra: FormExtra::NONE },
        FormField { name: "unreachable_fallback", description: Some("Behavior when Conduct is unreachable, times out, or rejects the token."), required: false, shape: FormShape::Choice(vec![json!("fail_open"), json!("fail_closed")]), default: Some(json!("fail_closed")), extra: FormExtra::NONE },
        FormField { name: "workspace_id", description: Some("Conduct workspace id, sent as the X-Workspace-Id header. Env: CONDUCT_WORKSPACE_ID."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
fn typesafe() -> FormModel {
    FormModel { ui_friendly_name: Some("TypeSafe (Jev) Compaction"), fields: vec![
        FormField { name: "api_base", description: Some("Base URL of the TypeSafe API. Falls back to the TYPESAFE_API_BASE env var, then https://api.typesafe.ai."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "api_key", description: Some("TypeSafe API key, sent as a Bearer token. Falls back to the TYPESAFE_API_KEY env var."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "model", description: Some("TypeSafe evaluation model (not the LLM). Defaults to 'jev-latest'."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "optional_params", description: Some("Optional parameters for the guardrail"), required: false, shape: FormShape::Nested(Box::new(typesafe_optional_params_model())), default: None, extra: FormExtra::NONE },
        FormField { name: "unreachable_fallback", description: Some("Behavior when the TypeSafe evaluation service is unreachable or errors. 'fail_open' (default) forwards the request uncompacted. 'fail_closed' raises an error instead."), required: false, shape: FormShape::Choice(vec![json!("fail_closed"), json!("fail_open")]), default: Some(json!("fail_open")), extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
fn typesafe_optional_params_model() -> FormModel {
    FormModel { ui_friendly_name: None, fields: vec![
        FormField { name: "max_result_chars_in_state", description: Some("Tool result text is truncated to this many characters when sent to the Jev evaluator, keeping the head and tail. Defaults to 4000."), required: false, shape: FormShape::Number, default: None, extra: FormExtra::NONE },
        FormField { name: "min_chars_to_evaluate", description: Some("Skip tool exchanges whose combined tool-result text is shorter than this many characters. Defaults to 200."), required: false, shape: FormShape::Number, default: None, extra: FormExtra::NONE },
        FormField { name: "relevance_threshold", description: Some("Relevance cutoff in [0, 1]. A completed tool exchange is dropped when Jev scores the probability that it is still needed below this value. Defaults to 0.2."), required: false, shape: FormShape::Number, default: None, extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
fn dynamoai() -> FormModel {
    FormModel { ui_friendly_name: Some("DynamoAI Guardrails"), fields: vec![
        FormField { name: "api_base", description: Some("Base URL for DynamoAI API. If not provided, the `DYNAMOAI_API_BASE` environment variable is checked, defaults to https://api.dynamo.ai"), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "api_key", description: Some("API key for DynamoAI Guardrails. If not provided, the `DYNAMOAI_API_KEY` environment variable is checked."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "guardrail_name", description: Some("Name of the guardrail for identification in logs and traces."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "model_id", description: Some("Model ID for tracking/logging purposes. If not provided, the `DYNAMOAI_MODEL_ID` environment variable is checked."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "policy_ids", description: Some("List of DynamoAI policy IDs to apply. If not provided, the `DYNAMOAI_POLICY_IDS` environment variable is checked (comma-separated)."), required: false, shape: FormShape::List { options: Vec::new() }, default: None, extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
fn headroom() -> FormModel {
    FormModel { ui_friendly_name: Some("Headroom"), fields: vec![
        FormField { name: "api_base", description: Some("Base URL for the headroom compression service (e.g. https://api.headroom.ai). Falls back to HEADROOM_API_BASE env var."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "api_key", description: Some("API key for the headroom compression service. Falls back to HEADROOM_API_KEY env var."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "ccr_retrieval", description: Some("Inject the Headroom retrieval tool for hashes declared by the compression service."), required: false, shape: FormShape::Boolean, default: Some(json!(true)), extra: FormExtra::NONE },
        FormField { name: "model", description: Some("Model name forwarded to the headroom /v1/compress endpoint."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "optional_params", description: Some("Optional parameters for the guardrail"), required: false, shape: FormShape::Other, default: None, extra: FormExtra::NONE },
        FormField { name: "unreachable_fallback", description: Some("Behavior when the headroom compression service is unreachable or errors. 'fail_closed' raises an error (default). 'fail_open' logs a critical error and forwards the request uncompressed instead of blocking it."), required: false, shape: FormShape::Choice(vec![json!("fail_closed"), json!("fail_open")]), default: Some(json!("fail_closed")), extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
fn promptguard() -> FormModel {
    FormModel { ui_friendly_name: Some("PromptGuard"), fields: vec![
        FormField { name: "api_base", description: Some("PromptGuard API base URL. Defaults to https://api.promptguard.co. Falls back to PROMPTGUARD_API_BASE env var."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "api_key", description: Some("API key for PromptGuard authentication. If not provided, the PROMPTGUARD_API_KEY environment variable is used."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "block_on_error", description: Some("Whether to block the request when the PromptGuard API is unreachable. Defaults to true (fail-closed). Set to false for fail-open behaviour."), required: false, shape: FormShape::Boolean, default: None, extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
fn qualifire() -> FormModel {
    FormModel { ui_friendly_name: Some("Qualifire"), fields: vec![
        FormField { name: "api_base", description: Some("The API base URL for Qualifire. If not provided, the `QUALIFIRE_BASE_URL` environment variable is checked."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "api_key", description: Some("The API key for Qualifire. If not provided, the `QUALIFIRE_API_KEY` environment variable is checked."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "assertions", description: Some("Custom assertions to validate against the output. Each assertion is a string describing a condition."), required: false, shape: FormShape::List { options: Vec::new() }, default: None, extra: FormExtra::NONE },
        FormField { name: "content_moderation_check", description: Some("Enable content moderation to check for harmful content (harassment, hate speech, etc.)."), required: false, shape: FormShape::Boolean, default: None, extra: FormExtra::NONE },
        FormField { name: "evaluation_id", description: Some("Pre-configured evaluation ID from Qualifire dashboard. When provided, uses invoke_evaluation() instead of evaluate()."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "grounding_check", description: Some("Enable grounding verification to ensure output is grounded in provided context."), required: false, shape: FormShape::Boolean, default: None, extra: FormExtra::NONE },
        FormField { name: "hallucinations_check", description: Some("Enable hallucination detection to detect factual inaccuracies."), required: false, shape: FormShape::Boolean, default: None, extra: FormExtra::NONE },
        FormField { name: "on_flagged", description: Some("Action to take when content is flagged. 'block' raises an exception, 'monitor' logs but allows the request."), required: false, shape: FormShape::Choice(vec![json!("block"), json!("monitor")]), default: Some(json!("block")), extra: FormExtra::NONE },
        FormField { name: "pii_check", description: Some("Enable PII (Personally Identifiable Information) detection."), required: false, shape: FormShape::Boolean, default: None, extra: FormExtra::NONE },
        FormField { name: "prompt_injections", description: Some("Enable prompt injection detection. Default check if no evaluation_id and no other checks are specified."), required: false, shape: FormShape::Boolean, default: None, extra: FormExtra::NONE },
        FormField { name: "tool_selection_quality_check", description: Some("Enable tool selection quality check to evaluate quality of tool/function calls."), required: false, shape: FormShape::Boolean, default: None, extra: FormExtra::NONE },
    ] }
}
