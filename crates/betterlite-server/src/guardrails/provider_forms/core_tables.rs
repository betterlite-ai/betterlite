use serde_json::{Value, json};

use crate::guardrails::provider_forms::spec::{FormExtra, FormField, FormModel, FormShape};

// LiteLLM builds the five route-owned entries before the registry loop adds every other provider
#[must_use]
pub fn core_tables() -> Vec<(&'static str, FormModel)> {
    vec![
        ("bedrock", bedrock_model()),
        ("presidio", presidio_model()),
        ("lakera_v2", lakera_v2_model()),
        ("tool_permission", tool_permission_model()),
        ("hide-secrets", hide_secrets_model()),
    ]
}

#[rustfmt::skip]
fn bedrock_model() -> FormModel {
    FormModel { ui_friendly_name: None, fields: vec![
            FormField { name: "aws_access_key_id", description: Some("AWS access key ID for authentication"), required: false, shape: FormShape::Text, default: None, extra: FormExtra { ui_type: None, type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
            FormField { name: "aws_bedrock_runtime_endpoint", description: Some("AWS Bedrock runtime endpoint URL"), required: false, shape: FormShape::Text, default: None, extra: FormExtra { ui_type: None, type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
            FormField { name: "aws_external_id", description: Some("External ID required by the target role's trust policy on sts:AssumeRole"), required: false, shape: FormShape::Text, default: None, extra: FormExtra { ui_type: None, type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
            FormField { name: "aws_profile_name", description: Some("AWS profile name for credential retrieval"), required: false, shape: FormShape::Text, default: None, extra: FormExtra { ui_type: None, type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
            FormField { name: "aws_region_name", description: Some("AWS region where your guardrail is deployed"), required: false, shape: FormShape::Text, default: None, extra: FormExtra { ui_type: None, type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
            FormField { name: "aws_role_name", description: Some("AWS role name for assuming roles"), required: false, shape: FormShape::Text, default: None, extra: FormExtra { ui_type: None, type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
            FormField { name: "aws_secret_access_key", description: Some("AWS secret access key for authentication"), required: false, shape: FormShape::Text, default: None, extra: FormExtra { ui_type: None, type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
            FormField { name: "aws_session_name", description: Some("Name of the AWS session"), required: false, shape: FormShape::Text, default: None, extra: FormExtra { ui_type: None, type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
            FormField { name: "aws_session_token", description: Some("AWS session token for temporary credentials"), required: false, shape: FormShape::Text, default: None, extra: FormExtra { ui_type: None, type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
            FormField { name: "aws_sts_endpoint", description: Some("AWS STS endpoint URL"), required: false, shape: FormShape::Text, default: None, extra: FormExtra { ui_type: None, type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
            FormField { name: "aws_web_identity_token", description: Some("Web identity token for AWS role assumption"), required: false, shape: FormShape::Text, default: None, extra: FormExtra { ui_type: None, type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
            FormField { name: "checks", description: Some("Inline safeguards for the resource-less InvokeGuardrailChecks API (contentFilter / promptAttack / sensitiveInformation). When set, the guardrail calls InvokeGuardrailChecks instead of ApplyGuardrail and no guardrailIdentifier is required. Mutually exclusive with guardrailIdentifier."), required: false, shape: FormShape::Nested(Box::new(bedrock_checks_model())), default: None, extra: FormExtra { ui_type: None, type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
            FormField { name: "chunk_budget_chars", description: Some("ApplyGuardrail: batch size, in characters, used to re-send content after AWS has rejected a request as too large. Requests AWS accepts are always sent in a single call, so this has no effect until a rejection happens. Defaults to 25,000; a batch AWS still rejects is bisected automatically, so this value only trades round trips against batch size and cannot fail a request on its own."), required: false, shape: FormShape::Number, default: Some(json!(25_000_i64)), extra: FormExtra { ui_type: None, type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
            FormField { name: "content_filter_threshold", description: Some("InvokeGuardrailChecks: block when any contentFilter severityScore >= this value (scores are in [0,1]). Set to null to make the content filter detect-only (logged, never blocks)."), required: false, shape: FormShape::Number, default: Some(json!(0.5_f64)), extra: FormExtra { ui_type: None, type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
            FormField { name: "contextual_grounding_from_messages", description: Some("ApplyGuardrail: when True, post-call scans of a request with no grounding_source / query content parts send the system and developer messages as the grounding source and the latest user message as the query, so the guardrail's contextual grounding policy can score the response. Bedrock bills contextual grounding units for these scans and rejects queries, sources and responses over its contextual grounding length limits, so leave this off for guardrails without a contextual grounding policy. Default False: plain messages are never sent as grounding context."), required: false, shape: FormShape::Boolean, default: Some(json!(false)), extra: FormExtra { ui_type: None, type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
            FormField { name: "disable_exception_on_block", description: Some("If True, will not raise an exception when the guardrail is blocked. Useful for OpenWebUI where exceptions can end the chat flow."), required: false, shape: FormShape::Boolean, default: Some(json!(false)), extra: FormExtra { ui_type: None, type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
            FormField { name: "guardrailIdentifier", description: Some("The ID of your guardrail on Bedrock"), required: false, shape: FormShape::Text, default: None, extra: FormExtra { ui_type: None, type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
            FormField { name: "guardrailVersion", description: Some("The version of your Bedrock guardrail (e.g., DRAFT or version number)"), required: false, shape: FormShape::Text, default: None, extra: FormExtra { ui_type: None, type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
            FormField { name: "pii_confidence_threshold", description: Some("InvokeGuardrailChecks: block when any sensitiveInformation confidenceScore >= this value (scores are in [0,1]). Set to null to make PII detection detect-only."), required: false, shape: FormShape::Number, default: Some(json!(0.5_f64)), extra: FormExtra { ui_type: None, type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
            FormField { name: "prompt_attack_threshold", description: Some("InvokeGuardrailChecks: block when any promptAttack severityScore >= this value (scores are in [0,1]). Set to null to make prompt-attack detection detect-only."), required: false, shape: FormShape::Number, default: Some(json!(0.5_f64)), extra: FormExtra { ui_type: None, type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
            FormField { name: "streaming_buffer_release_on_scan", description: Some("When buffering, scan the accumulated response every streaming_sampling_rate chunks and release the withheld chunks once the scan passes, instead of holding everything to end of stream. Flagged content is never released. Ignored when streaming_end_of_stream_only is true."), required: false, shape: FormShape::Boolean, default: Some(json!(false)), extra: FormExtra { ui_type: None, type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
            FormField { name: "streaming_buffer_until_moderated", description: Some("If True (default), withhold every streamed chunk until the end-of-stream ApplyGuardrail scan passes, so no flagged content reaches the client before a block. If False, chunks stream through unbuffered, so flagged content can reach the client before the scan finishes; a flagged scan still ends the stream, with a block message when disable_exception_on_block is true and an in-stream error frame otherwise."), required: false, shape: FormShape::Boolean, default: Some(json!(true)), extra: FormExtra { ui_type: None, type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
            FormField { name: "streaming_end_of_stream_only", description: Some("When not buffering, skip per-chunk sampling and run one ApplyGuardrail scan on the assembled response at end of stream. Combined with streaming_buffer_until_moderated=false the full response streams live before the scan and the scan result lands in guardrail_information; a flagged response still ends the stream with a block message (disable_exception_on_block=true) or an error frame."), required: false, shape: FormShape::Boolean, default: Some(json!(false)), extra: FormExtra { ui_type: None, type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
            FormField { name: "streaming_sampling_rate", description: Some("When not buffering and not end-of-stream-only, scan the accumulated response every Nth streamed chunk. Each sampled scan is a full ApplyGuardrail call that delays that chunk, so lower values add latency and AWS text-unit cost."), required: false, shape: FormShape::Number, default: Some(json!(5_i64)), extra: FormExtra { ui_type: None, type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
    ] }
}

#[rustfmt::skip]
fn bedrock_checks_model() -> FormModel {
    FormModel { ui_friendly_name: None, fields: vec![
            FormField { name: "contentFilter", description: None, required: false, shape: FormShape::Nested(Box::new(content_filter_model())), default: None, extra: FormExtra { ui_type: None, type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
            FormField { name: "promptAttack", description: None, required: false, shape: FormShape::Nested(Box::new(prompt_attack_model())), default: None, extra: FormExtra { ui_type: None, type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
            FormField { name: "sensitiveInformation", description: None, required: false, shape: FormShape::Nested(Box::new(sensitive_information_model())), default: None, extra: FormExtra { ui_type: None, type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
    ] }
}

#[rustfmt::skip]
fn content_filter_model() -> FormModel {
    FormModel { ui_friendly_name: None, fields: vec![
            FormField { name: "categories", description: None, required: true, shape: FormShape::List { options: Vec::new() }, default: Some(Value::Null), extra: FormExtra { ui_type: None, type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
    ] }
}

#[rustfmt::skip]
fn prompt_attack_model() -> FormModel {
    FormModel { ui_friendly_name: None, fields: vec![
            FormField { name: "categories", description: None, required: true, shape: FormShape::List { options: Vec::new() }, default: Some(Value::Null), extra: FormExtra { ui_type: None, type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
    ] }
}

#[rustfmt::skip]
fn sensitive_information_model() -> FormModel {
    FormModel { ui_friendly_name: None, fields: vec![
            FormField { name: "entities", description: None, required: true, shape: FormShape::List { options: Vec::new() }, default: Some(Value::Null), extra: FormExtra { ui_type: None, type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
    ] }
}

#[rustfmt::skip]
fn presidio_model() -> FormModel {
    FormModel { ui_friendly_name: None, fields: vec![
            FormField { name: "output_parse_pii", description: Some("When True, LiteLLM will replace the masked text with the original text in the response"), required: false, shape: FormShape::Boolean, default: None, extra: FormExtra { ui_type: Some("bool".to_owned()), type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
            FormField { name: "presidio_analyzer_api_base", description: Some("Base URL for the Presidio analyzer API"), required: false, shape: FormShape::Text, default: None, extra: FormExtra { ui_type: None, type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
            FormField { name: "presidio_anonymizer_api_base", description: Some("Base URL for the Presidio anonymizer API"), required: false, shape: FormShape::Text, default: None, extra: FormExtra { ui_type: None, type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
            FormField { name: "presidio_filter_scope", description: Some("Where to apply Presidio checks: 'input' (user -> model), 'output' (model -> user), or 'both' (default)."), required: false, shape: FormShape::Choice(vec![json!("input"), json!("output"), json!("both")]), default: None, extra: FormExtra { ui_type: None, type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
            FormField { name: "presidio_language", description: Some("Language code for Presidio PII analysis (e.g., 'en', 'de', 'es', 'fr')"), required: false, shape: FormShape::Text, default: Some(json!("en")), extra: FormExtra { ui_type: None, type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
            FormField { name: "presidio_run_on", description: Some("Where to apply Presidio checks: input, output, or both (default)."), required: false, shape: FormShape::Choice(vec![json!("input"), json!("output"), json!("both")]), default: None, extra: FormExtra { ui_type: None, type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
    ] }
}

#[rustfmt::skip]
fn lakera_v2_model() -> FormModel {
    FormModel { ui_friendly_name: None, fields: vec![
            FormField { name: "advisory_system_message", description: Some("Custom advisory message template used when on_flagged='inject_system_message'. Must contain a {reason} placeholder. Defaults to a generic advisory message if unset."), required: false, shape: FormShape::Text, default: None, extra: FormExtra { ui_type: None, type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
            FormField { name: "api_base", description: Some("Base URL for the Lakera AI API"), required: false, shape: FormShape::Text, default: None, extra: FormExtra { ui_type: None, type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
            FormField { name: "api_key", description: Some("API key for the Lakera AI service"), required: false, shape: FormShape::Text, default: None, extra: FormExtra { ui_type: None, type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
            FormField { name: "breakdown", description: Some("Whether to include breakdown in the response"), required: false, shape: FormShape::Boolean, default: Some(json!(true)), extra: FormExtra { ui_type: None, type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
            FormField { name: "dev_info", description: Some("Whether to include developer information in the response"), required: false, shape: FormShape::Boolean, default: Some(json!(true)), extra: FormExtra { ui_type: None, type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
            FormField { name: "metadata", description: Some("Additional metadata to include in the request"), required: false, shape: FormShape::RawObject, default: None, extra: FormExtra { ui_type: None, type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
            FormField { name: "on_flagged", description: Some("Action to take when content is flagged: 'block' (raise exception), 'monitor' (log only), or 'inject_system_message' (append an advisory system message and let the LLM decide)"), required: false, shape: FormShape::Choice(vec![json!("block"), json!("monitor"), json!("inject_system_message")]), default: Some(json!("block")), extra: FormExtra { ui_type: None, type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
            FormField { name: "payload", description: Some("Whether to include payload in the response"), required: false, shape: FormShape::Boolean, default: Some(json!(true)), extra: FormExtra { ui_type: None, type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
            FormField { name: "project_id", description: Some("Project ID for the Lakera AI project"), required: false, shape: FormShape::Text, default: None, extra: FormExtra { ui_type: None, type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
    ] }
}

#[rustfmt::skip]
fn tool_permission_model() -> FormModel {
    FormModel { ui_friendly_name: Some("LiteLLM Tool Permission Guardrail"), fields: vec![
            FormField { name: "default_action", description: Some("Fallback decision when no rule matches"), required: false, shape: FormShape::Choice(vec![json!("allow"), json!("deny")]), default: Some(json!("deny")), extra: FormExtra { ui_type: None, type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
            FormField { name: "on_disallowed_action", description: Some("Choose whether disallowed tools block the request or get rewritten out of the payload"), required: false, shape: FormShape::Choice(vec![json!("block"), json!("rewrite")]), default: Some(json!("block")), extra: FormExtra { ui_type: None, type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
            FormField { name: "rules", description: Some("Ordered allow/deny rules. Patterns use regex for tool names/types and optional regex constraints on tool arguments."), required: false, shape: FormShape::List { options: Vec::new() }, default: None, extra: FormExtra { ui_type: None, type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
    ] }
}

#[rustfmt::skip]
fn hide_secrets_model() -> FormModel {
    FormModel { ui_friendly_name: Some("Hide Secrets"), fields: vec![
            FormField { name: "detect_secrets_config", description: Some("Optional detect-secrets configuration (plugins_used, filters_used) overriding the bundled plugin set"), required: false, shape: FormShape::RawObject, default: None, extra: FormExtra { ui_type: None, type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
    ] }
}
