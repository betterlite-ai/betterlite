use serde_json::json;

use crate::guardrails::provider_forms::spec::{FormExtra, FormField, FormModel, FormShape};

#[rustfmt::skip]
pub(super) fn custom_code_model() -> FormModel {
    FormModel { ui_friendly_name: None, fields: vec![
        FormField { name: "custom_code", description: None, required: true, shape: FormShape::Text, default: Some(json!(null)), extra: FormExtra::NONE },
        FormField { name: "timeout", description: Some("Wall-clock limit in seconds for one run of apply_guardrail, module-level code included. A run that exceeds it fails the request instead of stalling the proxy."), required: false, shape: FormShape::Number, default: Some(json!(30.0_f64)), extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
pub(super) fn generic_guardrail_api_model() -> FormModel {
    FormModel { ui_friendly_name: Some("Generic Guardrail API"), fields: vec![
        FormField { name: "optional_params", description: Some("Optional parameters for the Generic Guardrail API guardrail"), required: false, shape: FormShape::Nested(Box::new(generic_guardrail_api_params())), default: None, extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
pub(super) fn generic_guardrail_api_params() -> FormModel {
    FormModel { ui_friendly_name: None, fields: vec![
        FormField { name: "additional_provider_specific_params", description: Some("Additional provider-specific parameters to send with the guardrail request"), required: false, shape: FormShape::Mapping { keys: Vec::new(), value: Box::new(FormShape::Text) }, default: None, extra: FormExtra::NONE },
        FormField { name: "fail_on_error", description: Some("Behavior on any guardrail error, not just unreachability. True (default) raises and blocks the request on error. False logs a critical error and allows the request to proceed, so only a valid guardrail response can block or modify it; broader than unreachable_fallback."), required: false, shape: FormShape::Boolean, default: Some(json!(true)), extra: FormExtra::NONE },
        FormField { name: "streaming_end_of_stream_only", description: Some("If False (default when unset), the guardrail runs on sampled chunks during the stream at the cadence set by streaming_sampling_rate, and an in-flight BLOCKED stops further chunks from streaming. If True, the guardrail runs once at end of stream over the assembled response; lower cost and latency, but flagged content has already streamed to the client before the terminal block. Defaults are applied in GenericGuardrailAPI.__init__ when None so unset optional_params does not shadow top-level litellm_params."), required: false, shape: FormShape::Boolean, default: None, extra: FormExtra::NONE },
        FormField { name: "streaming_sampling_rate", description: Some("When streaming_end_of_stream_only is False, the guardrail runs every Nth streamed chunk. Ignored when streaming_end_of_stream_only is True. Must be >= 1 when set. Defaults to 5 in GenericGuardrailAPI.__init__ when None so unset optional_params does not shadow top-level litellm_params."), required: false, shape: FormShape::Number, default: None, extra: FormExtra::NONE },
        FormField { name: "streaming_transform_mode", description: Some("Controls whether text modifications returned by the guardrail (action=GUARDRAIL_INTERVENED with modified texts) reach the client on the streaming path. 'block_only' (default) preserves the historical behavior: the raw upstream chunks are streamed and only a BLOCK terminates the stream; text rewrites are dropped. 'incremental_diff' withholds the raw chunks and instead emits the guardrailed text as new deltas computed by diffing the mutated accumulated text against what has already been sent, enabling PII masking, pseudonym reversal, redaction and similar rewrites over HTTP. Only supported for the OpenAI chat completions streaming path (string delta.content) and ignored when streaming_end_of_stream_only is True except for a single post-stream synthetic chunk. Defaults to 'block_only' in GenericGuardrailAPI.__init__ when None."), required: false, shape: FormShape::Choice(vec![json!("block_only"), json!("incremental_diff")]), default: None, extra: FormExtra::NONE },
        FormField { name: "unreachable_fallback", description: Some("Behavior when the guardrail endpoint is unreachable due to network errors. 'fail_closed' raises an error (default). 'fail_open' logs a critical error and allows the request to proceed."), required: false, shape: FormShape::Choice(vec![json!("fail_closed"), json!("fail_open")]), default: Some(json!("fail_closed")), extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
pub(super) fn tool_permission_model() -> FormModel {
    FormModel { ui_friendly_name: Some("LiteLLM Tool Permission Guardrail"), fields: vec![
        FormField { name: "default_action", description: Some("Fallback decision when no rule matches"), required: false, shape: FormShape::Choice(vec![json!("allow"), json!("deny")]), default: Some(json!("deny")), extra: FormExtra::NONE },
        FormField { name: "on_disallowed_action", description: Some("Choose whether disallowed tools block the request or get rewritten out of the payload"), required: false, shape: FormShape::Choice(vec![json!("block"), json!("rewrite")]), default: Some(json!("block")), extra: FormExtra::NONE },
        FormField { name: "rules", description: Some("Ordered allow/deny rules. Patterns use regex for tool names/types and optional regex constraints on tool arguments."), required: false, shape: FormShape::List { options: Vec::new() }, default: None, extra: FormExtra::NONE },
    ] }
}
