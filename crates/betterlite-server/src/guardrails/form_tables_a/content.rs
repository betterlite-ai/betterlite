use serde_json::json;

use crate::guardrails::provider_forms::spec::{FormExtra, FormField, FormModel, FormShape};

#[must_use]
pub(crate) fn tables() -> Vec<(&'static str, FormModel)> {
    vec![
        ("litellm_content_filter", litellm_content_filter()),
        ("zscaler_ai_guard", zscaler_ai_guard()),
        ("guardrails_ai", guardrails_ai()),
    ]
}

#[rustfmt::skip]
fn litellm_content_filter() -> FormModel {
    FormModel { ui_friendly_name: Some("LiteLLM Content Filter"), fields: vec![
        FormField { name: "blocked_words", description: Some("List of blocked keywords with actions"), required: false, shape: FormShape::List { options: Vec::new() }, default: None, extra: FormExtra::NONE },
        FormField { name: "blocked_words_file", description: Some("Path to YAML file containing blocked words"), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "categories", description: Some("List of prebuilt categories to enable (harmful_*, bias_*)"), required: false, shape: FormShape::List { options: Vec::new() }, default: None, extra: FormExtra::NONE },
        FormField { name: "competitor_intent_config", description: Some("Optional config for intent-based competitor comparison detection. Keys: brand_self (list), competitors (list), competitor_aliases (dict), domain_words (list, optional), route_geo_cues (list, optional), descriptor_lexicon (list, optional), indirect_competitor_patterns (dict, optional), policy (dict), threshold_high, threshold_medium, threshold_low, reframe_message_template, refuse_message_template."), required: false, shape: FormShape::Mapping { keys: Vec::new(), value: Box::new(FormShape::Other) }, default: None, extra: FormExtra::NONE },
        FormField { name: "keyword_redaction_tag", description: Some("Tag to use for keyword redaction"), required: false, shape: FormShape::Text, default: Some(json!("[KEYWORD_REDACTED]")), extra: FormExtra::NONE },
        FormField { name: "pattern_redaction_format", description: Some("Format string for pattern redaction (use {pattern_name} placeholder)"), required: false, shape: FormShape::Text, default: Some(json!("[{pattern_name}_REDACTED]")), extra: FormExtra::NONE },
        FormField { name: "patterns", description: Some("List of regex patterns to detect (prebuilt or custom)"), required: false, shape: FormShape::List { options: Vec::new() }, default: None, extra: FormExtra::NONE },
        FormField { name: "severity_threshold", description: Some("Minimum severity to block (high, medium, low)"), required: false, shape: FormShape::Text, default: Some(json!("medium")), extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
fn zscaler_ai_guard() -> FormModel {
    FormModel { ui_friendly_name: Some("Zscaler AI Guard"), fields: vec![
        FormField { name: "api_base", description: Some("Zscaler AI Guard API endpoint. Determines policy resolution behavior:\n• /execute-policy (default) - Requires explicit policy_id in configuration\n• /resolve-and-execute-policy - Infers policy from user-api-key-alias header\nDefault: https://api.us1.zseclipse.net/v1/detection/execute-policy\nFalls back to ZSCALER_AI_GUARD_URL environment variable."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "api_key", description: Some("API key for Zscaler AI Guard authentication. If not provided, falls back to ZSCALER_AI_GUARD_API_KEY environment variable."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "policy_id", description: Some("Global policy ID for Zscaler AI Guard. Required when using /execute-policy endpoint.\n\nSet to 0 or leave empty when using /resolve-and-execute-policy with dynamic policy resolution.\nFalls back to ZSCALER_AI_GUARD_POLICY_ID environment variable."), required: false, shape: FormShape::Number, default: None, extra: FormExtra::NONE },
        FormField { name: "send_user_api_key_alias", description: Some("Send user API key alias in request headers as 'user-api-key-alias'. CRITICAL when using /resolve-and-execute-policy endpoint - the policy is inferred from this value. Also useful for tracking/auditing with /execute-policy endpoint."), required: false, shape: FormShape::Boolean, default: Some(json!(false)), extra: FormExtra { ui_type: Some("bool".to_owned()), type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
        FormField { name: "send_user_api_key_team_id", description: Some("Send user API key team_id in request headers as 'user-api-key-team-id'. Enables team-level tracking and analytics in Zscaler AI Guard."), required: false, shape: FormShape::Boolean, default: Some(json!(false)), extra: FormExtra { ui_type: Some("bool".to_owned()), type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
        FormField { name: "send_user_api_key_user_id", description: Some("Send user API key user_id in request headers as 'user-api-key-user-id'. Enables user-level tracking and analytics in Zscaler AI Guard."), required: false, shape: FormShape::Boolean, default: Some(json!(false)), extra: FormExtra { ui_type: Some("bool".to_owned()), type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
        FormField { name: "timeout", description: Some("Timeout for each Zscaler AI Guard API call, in seconds. Must be positive. Raise it if scans fail under load with 'Connection timed out'. Defaults to 5 seconds."), required: false, shape: FormShape::Number, default: None, extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
fn guardrails_ai() -> FormModel {
    FormModel { ui_friendly_name: Some("Guardrails AI"), fields: vec![
        FormField { name: "api_base", description: Some("The API base for the Guardrails AI guardrail. Defaults to http://0.0.0.0:8000, the `GUARDRAILS_AI_API_BASE` environment variable is checked."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "guard_name", description: Some("The name of the Guardrails AI guardrail. Required for the Guardrails AI guardrail."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "guardrails_ai_api_input_format", description: Some("The format of the input to the Guardrails AI API. Defaults to 'llmOutput'."), required: false, shape: FormShape::Choice(vec![json!("inputs"), json!("llmOutput")]), default: Some(json!("llmOutput")), extra: FormExtra::NONE },
    ] }
}
