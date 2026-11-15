use serde_json::json;

use crate::guardrails::provider_forms::spec::{FormExtra, FormField, FormModel, FormShape};

#[rustfmt::skip]
pub(super) fn aporia_model() -> FormModel {
    FormModel { ui_friendly_name: Some("Aporia AI"), fields: vec![
        FormField { name: "api_base", description: Some("The API base for the Aporia guardrail. If not provided, the `APORIA_API_BASE` environment variable is checked."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "api_key", description: Some("The API key for the Aporia guardrail. If not provided, the `APORIA_API_KEY` environment variable is checked."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
pub(super) fn compresr_model() -> FormModel {
    FormModel { ui_friendly_name: Some("Compresr (context compression)"), fields: vec![
        FormField { name: "api_base", description: Some("Base URL of the Compresr API. Falls back to the COMPRESR_API_BASE env var, then https://api.compresr.ai. Point at your internal service URL for on-prem deployments."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "api_key", description: Some("Compresr API key. Falls back to the COMPRESR_API_KEY env var."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "model", description: Some("Compresr compression model (not the LLM). Defaults to 'latte_v2', the query-aware compression model."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "optional_params", description: Some("Optional parameters for the guardrail"), required: false, shape: FormShape::Nested(Box::new(compresr_params())), default: None, extra: FormExtra::NONE },
        FormField { name: "unreachable_fallback", description: Some("Behavior when the Compresr compression service is unreachable or errors. 'fail_closed' raises an error (default). 'fail_open' logs a critical error and forwards the request uncompressed instead of blocking it."), required: false, shape: FormShape::Choice(vec![json!("fail_closed"), json!("fail_open")]), default: Some(json!("fail_closed")), extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
pub(super) fn compresr_params() -> FormModel {
    FormModel { ui_friendly_name: None, fields: vec![
        FormField { name: "allow_bypass_header", description: Some("Honor the `x-compresr-bypass: true` request header to skip compression for a single call. Off by default because the header is caller-settable; enable only on trusted deployments."), required: false, shape: FormShape::Boolean, default: None, extra: FormExtra::NONE },
        FormField { name: "coarse", description: Some("Paragraph-level compression (default, faster) instead of token-level (finer-grained)."), required: false, shape: FormShape::Boolean, default: None, extra: FormExtra::NONE },
        FormField { name: "compress_history", description: Some("Also compress prior (non-last) user messages. Defaults to False."), required: false, shape: FormShape::Boolean, default: None, extra: FormExtra::NONE },
        FormField { name: "compress_last_user", description: Some("Also compress the last user message. The query sent to Compresr is always the original verbatim text. Defaults to False."), required: false, shape: FormShape::Boolean, default: None, extra: FormExtra::NONE },
        FormField { name: "compress_system", description: Some("Also compress system messages. Defaults to False."), required: false, shape: FormShape::Boolean, default: None, extra: FormExtra::NONE },
        FormField { name: "compress_tool_outputs", description: Some("Compress tool/function result messages (search hits, RAG chunks, API dumps). Defaults to True."), required: false, shape: FormShape::Boolean, default: None, extra: FormExtra::NONE },
        FormField { name: "compression_params", description: Some("Passthrough of extra parameters forwarded verbatim in the Compresr compress payload (e.g. `heuristic_chunking`, or any newer knob), so a new Compresr feature works without a guardrail update. The named fields above take precedence on collision."), required: false, shape: FormShape::Mapping { keys: Vec::new(), value: Box::new(FormShape::Text) }, default: None, extra: FormExtra::NONE },
        FormField { name: "dynamic", description: Some("latte_v2 only. Let the server choose the compression amount per input (Kneedle elbow) instead of using target_compression_ratio. Defaults to True."), required: false, shape: FormShape::Boolean, default: None, extra: FormExtra::NONE },
        FormField { name: "dynamic_max_ratio", description: Some("latte_v2 only. Ceiling on the adaptive ratio when `dynamic` is on. Unset lets the server default apply (~10.0)."), required: false, shape: FormShape::Number, default: None, extra: FormExtra::NONE },
        FormField { name: "dynamic_min_ratio", description: Some("latte_v2 only. Floor on the adaptive ratio when `dynamic` is on. Unset lets the server default apply (~1.5)."), required: false, shape: FormShape::Number, default: None, extra: FormExtra::NONE },
        FormField { name: "enable_retrieval", description: Some("Make compression recoverable: inject a `compresr_retrieve` tool so the model can fetch the original content behind a compression marker via the agentic loop. Defaults to True. Set to False (or run the proxy with --workers 1) for multi-worker deployments: the recovery store is per-process, so pre-call and retrieval hooks on different workers cannot see each other's originals."), required: false, shape: FormShape::Boolean, default: None, extra: FormExtra::NONE },
        FormField { name: "max_bytes_per_call", description: Some("Cap on aggregate bytes of stored originals per litellm_call_id. When a call exceeds this, oldest entries are evicted so the in-process store cannot grow without bound. Defaults to 10 MiB."), required: false, shape: FormShape::Number, default: None, extra: FormExtra::NONE },
        FormField { name: "min_chars_to_compress", description: Some("Skip messages whose text is shorter than this many characters. Defaults to 500."), required: false, shape: FormShape::Number, default: None, extra: FormExtra::NONE },
        FormField { name: "target_compression_ratio", description: Some("Compression strength. 0-1 is the fraction of tokens to remove (0.5 = remove ~50%, the default); a value >1 is an Nx reduction factor (e.g. 4 = ~4x smaller)."), required: false, shape: FormShape::Number, default: None, extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
pub(super) fn deepkeep_model() -> FormModel {
    FormModel { ui_friendly_name: Some("DeepKeep AI Firewall"), fields: vec![
        FormField { name: "api_base", description: Some("The API base URL for the DeepKeep AI Firewall. If not provided, the `DEEPKEEP_API_BASE` environment variable is checked."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "api_key", description: Some("The API key for the DeepKeep AI Firewall. If not provided, the `DEEPKEEP_API_KEY` environment variable is checked."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "deepkeep_firewall_id", description: Some("The DeepKeep Firewall ID to use for guardrail evaluation. If not provided, the `DEEPKEEP_FIREWALL_ID` environment variable is checked."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "optional_params", description: Some("Optional parameters for the guardrail"), required: false, shape: FormShape::Nested(Box::new(deepkeep_params())), default: None, extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
pub(super) fn deepkeep_params() -> FormModel {
    FormModel { ui_friendly_name: None, fields: vec![
        FormField { name: "unreachable_fallback", description: Some("Behavior when the DeepKeep API is unreachable. 'fail_closed' raises an error (default). 'fail_open' logs a critical error and allows the request to proceed."), required: false, shape: FormShape::Text, default: Some(json!("fail_closed")), extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
pub(super) fn enkryptai_model() -> FormModel {
    FormModel { ui_friendly_name: Some("EnkryptAI"), fields: vec![
        FormField { name: "api_base", description: Some("The EnkryptAI API base URL. Defaults to https://api.enkryptai.com. Also checks if the ENKRYPTAI_API_KEY env var is set."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "api_key", description: Some("The EnkryptAI API key. Reads from ENKRYPTAI_API_KEY env var if None."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "block_on_violation", description: Some("Whether to block requests when violations are detected. Defaults to True."), required: false, shape: FormShape::Boolean, default: Some(json!(true)), extra: FormExtra::NONE },
        FormField { name: "deployment_name", description: Some("The EnkryptAI deployment name to use. Sent via X-Enkrypt-Deployment header."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "detectors", description: Some("Dictionary of detector configurations (e.g., {'nsfw': {'enabled': True}, 'toxicity': {'enabled': True}})."), required: false, shape: FormShape::RawObject, default: None, extra: FormExtra::NONE },
        FormField { name: "policy_name", description: Some("The EnkryptAI policy name to use. Sent via x-enkrypt-policy header."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
pub(super) fn lasso_model() -> FormModel {
    FormModel { ui_friendly_name: Some("Lasso Guardrail"), fields: vec![
        FormField { name: "api_base", description: Some("The API base for the Lasso guardrail. Default is https://server.lasso.security. Also checks if the `LASSO_API_BASE` environment variable is set."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "api_key", description: Some("The API key for the Lasso guardrail. If not provided, the `LASSO_API_KEY` environment variable is checked."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "optional_params", description: Some("Optional parameters for the guardrail"), required: false, shape: FormShape::Nested(Box::new(lasso_params())), default: None, extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
pub(super) fn lasso_params() -> FormModel {
    FormModel { ui_friendly_name: None, fields: vec![
        FormField { name: "conversation_id", description: Some("The conversation ID for the Lasso guardrail. If not provided, the `LASSO_CONVERSATION_ID` environment variable is checked."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "user_id", description: Some("The user ID for the Lasso guardrail. If not provided, the `LASSO_USER_ID` environment variable is checked."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
pub(super) fn onyx_model() -> FormModel {
    FormModel { ui_friendly_name: Some("Onyx Guardrail"), fields: vec![
        FormField { name: "api_base", description: Some("The URL of the Onyx Guard server. If not provided, the `ONYX_API_BASE` environment variable is checked."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "api_key", description: Some("The API key for the Onyx Guard server. If not provided, the `ONYX_API_KEY` environment variable is checked."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "timeout", description: Some("The timeout for the Onyx Guard server in seconds. If not provided, the `ONYX_TIMEOUT` environment variable is checked."), required: false, shape: FormShape::Number, default: None, extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
pub(super) fn ovalix_model() -> FormModel {
    FormModel { ui_friendly_name: Some("Ovalix Guardrail"), fields: vec![
        FormField { name: "application_id", description: Some("Application ID for the Ovalix Tracker service."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "post_checkpoint_id", description: Some("Post-checkpoint ID for the Ovalix Tracker service."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "pre_checkpoint_id", description: Some("Pre-checkpoint ID for the Ovalix Tracker service."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "tracker_api_base", description: Some("Base URL for the Ovalix Tracker service."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "tracker_api_key", description: Some("API key for the Ovalix Tracker service."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
pub(super) fn pillar_model() -> FormModel {
    FormModel { ui_friendly_name: Some("Pillar Guardrail"), fields: vec![
        FormField { name: "api_base", description: Some("Base URL for the Pillar Security API. If not provided, the `PILLAR_API_BASE` environment variable is checked, defaults to https://api.pillar.security"), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "api_key", description: Some("API key for the Pillar Security service. If not provided, the `PILLAR_API_KEY` environment variable is checked."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "optional_params", description: Some("Optional parameters for the guardrail"), required: false, shape: FormShape::Nested(Box::new(pillar_params())), default: None, extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
pub(super) fn pillar_params() -> FormModel {
    FormModel { ui_friendly_name: None, fields: vec![
        FormField { name: "async_mode", description: Some("Set to True to request asynchronous analysis (sets `plr_async` header)."), required: false, shape: FormShape::Boolean, default: None, extra: FormExtra::NONE },
        FormField { name: "fallback_on_error", description: Some("Action to take when Pillar API is unavailable or errors: 'allow' (proceed without scanning) or 'block' (reject request with 503 error). If not provided, the `PILLAR_FALLBACK_ON_ERROR` environment variable is checked, defaults to 'allow'."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "include_evidence", description: Some("Include detailed evidence objects in response payloads (sets `plr_evidence` header)."), required: false, shape: FormShape::Boolean, default: Some(json!(true)), extra: FormExtra::NONE },
        FormField { name: "include_scanners", description: Some("Include scanner summaries in response payloads (sets `plr_scanners` header)."), required: false, shape: FormShape::Boolean, default: Some(json!(true)), extra: FormExtra::NONE },
        FormField { name: "on_flagged_action", description: Some("Action to take when content is flagged: 'block' (raise exception) or 'monitor' (log only). If not provided, the `PILLAR_ON_FLAGGED_ACTION` environment variable is checked, defaults to 'monitor'."), required: false, shape: FormShape::Text, default: Some(json!("monitor")), extra: FormExtra::NONE },
        FormField { name: "persist_session", description: Some("Set to False to disable session persistence (sets `plr_persist` header)."), required: false, shape: FormShape::Boolean, default: None, extra: FormExtra::NONE },
        FormField { name: "timeout", description: Some("Timeout in seconds for Pillar API calls. If not provided, the `PILLAR_TIMEOUT` environment variable is checked, defaults to 5.0 seconds."), required: false, shape: FormShape::Number, default: None, extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
pub(super) fn prompt_security_model() -> FormModel {
    FormModel { ui_friendly_name: Some("Prompt Security"), fields: vec![
        FormField { name: "api_base", description: Some("The API base for the Prompt Security guardrail. If not provided, the `PROMPT_SECURITY_API_BASE` environment variable is used."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "api_key", description: Some("The API key for the Prompt Security guardrail. If not provided, the `PROMPT_SECURITY_API_KEY` environment variable is used."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "block_on_file_modify", description: Some("Whether a file sanitization `modify` verdict blocks the request instead of replacing the file content."), required: false, shape: FormShape::Boolean, default: Some(json!(true)), extra: FormExtra::NONE },
        FormField { name: "file_sanitization_fail_open", description: Some("Whether file sanitization timeouts allow the original file through instead of blocking the request."), required: false, shape: FormShape::Boolean, default: Some(json!(true)), extra: FormExtra::NONE },
        FormField { name: "streaming_transform_mode", description: Some("How post_call `modify` verdicts reach a streaming client. `block_only` (default) streams the raw upstream chunks and only a `block` verdict ends the stream, so `modified_text` is dropped. `incremental_diff` buffers the whole response and sends the redacted text once the final verdict is in, so the first token arrives with the last, while a `block` verdict still ends the stream early. OpenAI chat completions streaming only."), required: false, shape: FormShape::Choice(vec![json!("block_only"), json!("incremental_diff")]), default: None, extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
pub(super) fn repelloai_model() -> FormModel {
    FormModel { ui_friendly_name: Some("RepelloAI Argus"), fields: vec![
        FormField { name: "api_base", description: Some("Base URL for the RepelloAI Argus API. Defaults to https://argusapi.repello.ai/sdk/v1"), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "api_key", description: Some("API key for the RepelloAI Argus service. Falls back to ARGUS_API_KEY or REPELLOAI_API_KEY."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "asset_id", description: Some("Repello asset ID whose dashboard policies are enforced. Required; the guardrail raises at init if it is missing."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "optional_params", description: Some("Optional parameters for the guardrail"), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "unreachable_fallback", description: Some("What to do when the RepelloAI Argus API is unreachable. 'fail_closed' = block (default), 'fail_open' = allow."), required: false, shape: FormShape::Choice(vec![json!("fail_closed"), json!("fail_open")]), default: Some(json!("fail_closed")), extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
pub(super) fn straiker_model() -> FormModel {
    FormModel { ui_friendly_name: Some("Straiker"), fields: vec![
        FormField { name: "api_base", description: Some("Straiker API base URL. Use the regional variant for non-US tenants."), required: false, shape: FormShape::Text, default: Some(json!("https://api.prod.straiker.ai")), extra: FormExtra::NONE },
        FormField { name: "api_key", description: Some("Straiker DefendAI environment API key (Bearer token). Env: STRAIKER_API_KEY."), required: true, shape: FormShape::Text, default: Some(json!(null)), extra: FormExtra::NONE },
        FormField { name: "default_app", description: Some("Default application registered in the Straiker Defend Console. Overridden per-request by metadata.agent_id when present."), required: false, shape: FormShape::Text, default: Some(json!("LiteLLM Gateway")), extra: FormExtra::NONE },
        FormField { name: "optional_params", description: Some("Optional parameters for the guardrail"), required: false, shape: FormShape::Nested(Box::new(straiker_params())), default: None, extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
pub(super) fn straiker_params() -> FormModel {
    FormModel { ui_friendly_name: None, fields: vec![
        FormField { name: "agent_ref", description: Some("v3 only. Names the Straiker agent this route's traffic belongs to when one gateway fronts several applications, sent as x-s6r-agent. It wins over a client-supplied x-s6r-agent header. Names ONE agent, never a kind of agent: Straiker keys per-agent state on it, so sharing a value across applications merges them into one agent."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "api_version", description: Some("Straiker detect API the gateway calls. 'v1' posts the structured webhook envelope to /api/v1/detect/webhook (legacy Defend, UUID collection key). 'v3' relays the provider request and response to /api/v3/detect, the v3 platform's only detect route, which accepts only an sk_agt_ integration key. Unset: chosen from the key prefix, so a v3 key needs no extra configuration."), required: false, shape: FormShape::Choice(vec![json!("v1"), json!("v3")]), default: None, extra: FormExtra::NONE },
        FormField { name: "client", description: Some("v3 only. Optional x-s6r-client routing hint. Leave unset on a shared gateway; set it on a route that serves a single application."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "custom_headers", description: Some("Additional HTTP headers sent to Straiker, excluding Authorization and the webhook-format header."), required: false, shape: FormShape::Mapping { keys: Vec::new(), value: Box::new(FormShape::Text) }, default: None, extra: FormExtra::NONE },
        FormField { name: "fail_on_error", description: Some("Behavior on any guardrail error, not just unreachability. True (default) blocks the request on error; False logs and allows the request to proceed."), required: false, shape: FormShape::Boolean, default: Some(json!(true)), extra: FormExtra::NONE },
        FormField { name: "format_hint", description: Some("v3 only. Optional x-s6r-format hint. Only breaks the messages-array tie between formats."), required: false, shape: FormShape::Choice(vec![json!("anthropic.messages"), json!("openai.chat")]), default: None, extra: FormExtra::NONE },
        FormField { name: "initial_backoff", description: Some("Initial retry backoff in seconds."), required: false, shape: FormShape::Number, default: Some(json!(0.1_f64)), extra: FormExtra::NONE },
        FormField { name: "max_backoff", description: Some("Maximum retry backoff in seconds."), required: false, shape: FormShape::Number, default: Some(json!(2.0_f64)), extra: FormExtra::NONE },
        FormField { name: "max_payload_bytes", description: Some("Maximum serialized webhook payload size sent to Straiker."), required: false, shape: FormShape::Number, default: Some(json!(524_288_i64)), extra: FormExtra::NONE },
        FormField { name: "max_retries", description: Some("Retries on transient HTTP (408/429/5xx) and network errors."), required: false, shape: FormShape::Number, default: Some(json!(2_i64)), extra: FormExtra::NONE },
        FormField { name: "metadata", description: Some("Default metadata key/values added to the webhook metadata bag on every request. On key conflict with request-derived metadata, these configured values win."), required: false, shape: FormShape::Mapping { keys: Vec::new(), value: Box::new(FormShape::Text) }, default: None, extra: FormExtra::NONE },
        FormField { name: "timeout", description: Some("Per-attempt HTTP timeout in seconds."), required: false, shape: FormShape::Number, default: Some(json!(5.0_f64)), extra: FormExtra::NONE },
        FormField { name: "unreachable_fallback", description: Some("Behavior when Straiker is unreachable after retries."), required: false, shape: FormShape::Choice(vec![json!("fail_open"), json!("fail_closed")]), default: Some(json!("fail_closed")), extra: FormExtra::NONE },
        FormField { name: "verbose", description: Some("Log webhook request/response payloads and record action/turn_id in response hidden params."), required: false, shape: FormShape::Boolean, default: Some(json!(false)), extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
pub(super) fn vigil_guard_model() -> FormModel {
    FormModel { ui_friendly_name: Some("Vigil Guard"), fields: vec![
        FormField { name: "api_base", description: Some("Vigil Guard API base URL. Falls back to the VIGIL_GUARD_URL environment variable."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "api_key", description: Some("Vigil Guard API key. Falls back to the VIGIL_GUARD_API_KEY environment variable."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
    ] }
}
