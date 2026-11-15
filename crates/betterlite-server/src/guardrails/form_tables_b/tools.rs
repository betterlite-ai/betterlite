use super::data::{SpecField, SpecModel, SpecShape, SpecValue};

#[rustfmt::skip]
pub(super) const CUSTOM_CODE: SpecModel = SpecModel { ui_friendly_name: None, fields: &[
    SpecField { name: "custom_code", description: None, required: true, shape: SpecShape::Text, default: Some(SpecValue::Null), ui_type: None },
    SpecField { name: "timeout", description: Some("Wall-clock limit in seconds for one run of apply_guardrail, module-level code included. A run that exceeds it fails the request instead of stalling the proxy."), required: false, shape: SpecShape::Number, default: Some(SpecValue::Float(30.0)), ui_type: None },
] };

#[rustfmt::skip]
pub(super) const GENERIC_GUARDRAIL_API: SpecModel = SpecModel { ui_friendly_name: Some("Generic Guardrail API"), fields: &[
    SpecField { name: "optional_params", description: Some("Optional parameters for the Generic Guardrail API guardrail"), required: false, shape: SpecShape::Nested(GENERIC_GUARDRAIL_API_PARAMS), default: None, ui_type: None },
] };

#[rustfmt::skip]
pub(super) const GENERIC_GUARDRAIL_API_PARAMS: &[SpecField] = &[
    SpecField { name: "additional_provider_specific_params", description: Some("Additional provider-specific parameters to send with the guardrail request"), required: false, shape: SpecShape::Mapping { keys: &[], value: &SpecShape::Text }, default: None, ui_type: None },
    SpecField { name: "fail_on_error", description: Some("Behavior on any guardrail error, not just unreachability. True (default) raises and blocks the request on error. False logs a critical error and allows the request to proceed, so only a valid guardrail response can block or modify it; broader than unreachable_fallback."), required: false, shape: SpecShape::Boolean, default: Some(SpecValue::Bool(true)), ui_type: None },
    SpecField { name: "streaming_end_of_stream_only", description: Some("If False (default when unset), the guardrail runs on sampled chunks during the stream at the cadence set by streaming_sampling_rate, and an in-flight BLOCKED stops further chunks from streaming. If True, the guardrail runs once at end of stream over the assembled response; lower cost and latency, but flagged content has already streamed to the client before the terminal block. Defaults are applied in GenericGuardrailAPI.__init__ when None so unset optional_params does not shadow top-level litellm_params."), required: false, shape: SpecShape::Boolean, default: None, ui_type: None },
    SpecField { name: "streaming_sampling_rate", description: Some("When streaming_end_of_stream_only is False, the guardrail runs every Nth streamed chunk. Ignored when streaming_end_of_stream_only is True. Must be >= 1 when set. Defaults to 5 in GenericGuardrailAPI.__init__ when None so unset optional_params does not shadow top-level litellm_params."), required: false, shape: SpecShape::Number, default: None, ui_type: None },
    SpecField { name: "streaming_transform_mode", description: Some("Controls whether text modifications returned by the guardrail (action=GUARDRAIL_INTERVENED with modified texts) reach the client on the streaming path. 'block_only' (default) preserves the historical behavior: the raw upstream chunks are streamed and only a BLOCK terminates the stream; text rewrites are dropped. 'incremental_diff' withholds the raw chunks and instead emits the guardrailed text as new deltas computed by diffing the mutated accumulated text against what has already been sent, enabling PII masking, pseudonym reversal, redaction and similar rewrites over HTTP. Only supported for the OpenAI chat completions streaming path (string delta.content) and ignored when streaming_end_of_stream_only is True except for a single post-stream synthetic chunk. Defaults to 'block_only' in GenericGuardrailAPI.__init__ when None."), required: false, shape: SpecShape::Choice(&["block_only", "incremental_diff"]), default: None, ui_type: None },
    SpecField { name: "unreachable_fallback", description: Some("Behavior when the guardrail endpoint is unreachable due to network errors. 'fail_closed' raises an error (default). 'fail_open' logs a critical error and allows the request to proceed."), required: false, shape: SpecShape::Choice(&["fail_closed", "fail_open"]), default: Some(SpecValue::Text("fail_closed")), ui_type: None },
];

#[rustfmt::skip]
pub(super) const TOOL_PERMISSION: SpecModel = SpecModel { ui_friendly_name: Some("LiteLLM Tool Permission Guardrail"), fields: &[
    SpecField { name: "default_action", description: Some("Fallback decision when no rule matches"), required: false, shape: SpecShape::Choice(&["allow", "deny"]), default: Some(SpecValue::Text("deny")), ui_type: None },
    SpecField { name: "on_disallowed_action", description: Some("Choose whether disallowed tools block the request or get rewritten out of the payload"), required: false, shape: SpecShape::Choice(&["block", "rewrite"]), default: Some(SpecValue::Text("block")), ui_type: None },
    SpecField { name: "rules", description: Some("Ordered allow/deny rules. Patterns use regex for tool names/types and optional regex constraints on tool arguments."), required: false, shape: SpecShape::List(&[]), default: None, ui_type: None },
] };
