use crate::guardrails::form_tables_a::model::{Extra, Field, Initial, Model, Shape};

pub(crate) const TABLES: &[(&str, Model)] = &[
    ("litellm_content_filter", LITELLM_CONTENT_FILTER),
    ("zscaler_ai_guard", ZSCALER_AI_GUARD),
    ("guardrails_ai", GUARDRAILS_AI),
];

#[rustfmt::skip]
const LITELLM_CONTENT_FILTER: Model = Model { name: Some("LiteLLM Content Filter"), fields: &[
    Field { name: "blocked_words", description: "List of blocked keywords with actions", required: false, shape: Shape::List(&[]), default: Initial::None, extra: Extra::NONE },
    Field { name: "blocked_words_file", description: "Path to YAML file containing blocked words", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
    Field { name: "categories", description: "List of prebuilt categories to enable (harmful_*, bias_*)", required: false, shape: Shape::List(&[]), default: Initial::None, extra: Extra::NONE },
    Field { name: "competitor_intent_config", description: "Optional config for intent-based competitor comparison detection. Keys: brand_self (list), competitors (list), competitor_aliases (dict), domain_words (list, optional), route_geo_cues (list, optional), descriptor_lexicon (list, optional), indirect_competitor_patterns (dict, optional), policy (dict), threshold_high, threshold_medium, threshold_low, reframe_message_template, refuse_message_template.", required: false, shape: Shape::Mapping(&[], &Shape::Other), default: Initial::None, extra: Extra::NONE },
    Field { name: "keyword_redaction_tag", description: "Tag to use for keyword redaction", required: false, shape: Shape::Text, default: Initial::Text("[KEYWORD_REDACTED]"), extra: Extra::NONE },
    Field { name: "pattern_redaction_format", description: "Format string for pattern redaction (use {pattern_name} placeholder)", required: false, shape: Shape::Text, default: Initial::Text("[{pattern_name}_REDACTED]"), extra: Extra::NONE },
    Field { name: "patterns", description: "List of regex patterns to detect (prebuilt or custom)", required: false, shape: Shape::List(&[]), default: Initial::None, extra: Extra::NONE },
    Field { name: "severity_threshold", description: "Minimum severity to block (high, medium, low)", required: false, shape: Shape::Text, default: Initial::Text("medium"), extra: Extra::NONE },
] };

#[rustfmt::skip]
const ZSCALER_AI_GUARD: Model = Model { name: Some("Zscaler AI Guard"), fields: &[
    Field { name: "api_base", description: "Zscaler AI Guard API endpoint. Determines policy resolution behavior:\n\u{2022} /execute-policy (default) - Requires explicit policy_id in configuration\n\u{2022} /resolve-and-execute-policy - Infers policy from user-api-key-alias header\nDefault: https://api.us1.zseclipse.net/v1/detection/execute-policy\nFalls back to ZSCALER_AI_GUARD_URL environment variable.", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
    Field { name: "api_key", description: "API key for Zscaler AI Guard authentication. If not provided, falls back to ZSCALER_AI_GUARD_API_KEY environment variable.", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
    Field { name: "policy_id", description: "Global policy ID for Zscaler AI Guard. Required when using /execute-policy endpoint.\n\nSet to 0 or leave empty when using /resolve-and-execute-policy with dynamic policy resolution.\nFalls back to ZSCALER_AI_GUARD_POLICY_ID environment variable.", required: false, shape: Shape::Number, default: Initial::None, extra: Extra::NONE },
    Field { name: "send_user_api_key_alias", description: "Send user API key alias in request headers as 'user-api-key-alias'. CRITICAL when using /resolve-and-execute-policy endpoint - the policy is inferred from this value. Also useful for tracking/auditing with /execute-policy endpoint.", required: false, shape: Shape::Boolean, default: Initial::Bool(false), extra: Extra { ui_type: Some("bool"), ..Extra::NONE } },
    Field { name: "send_user_api_key_team_id", description: "Send user API key team_id in request headers as 'user-api-key-team-id'. Enables team-level tracking and analytics in Zscaler AI Guard.", required: false, shape: Shape::Boolean, default: Initial::Bool(false), extra: Extra { ui_type: Some("bool"), ..Extra::NONE } },
    Field { name: "send_user_api_key_user_id", description: "Send user API key user_id in request headers as 'user-api-key-user-id'. Enables user-level tracking and analytics in Zscaler AI Guard.", required: false, shape: Shape::Boolean, default: Initial::Bool(false), extra: Extra { ui_type: Some("bool"), ..Extra::NONE } },
    Field { name: "timeout", description: "Timeout for each Zscaler AI Guard API call, in seconds. Must be positive. Raise it if scans fail under load with 'Connection timed out'. Defaults to 5 seconds.", required: false, shape: Shape::Number, default: Initial::None, extra: Extra::NONE },
] };

#[rustfmt::skip]
const GUARDRAILS_AI: Model = Model { name: Some("Guardrails AI"), fields: &[
    Field { name: "api_base", description: "The API base for the Guardrails AI guardrail. Defaults to http://0.0.0.0:8000, the `GUARDRAILS_AI_API_BASE` environment variable is checked.", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
    Field { name: "guard_name", description: "The name of the Guardrails AI guardrail. Required for the Guardrails AI guardrail.", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
    Field { name: "guardrails_ai_api_input_format", description: "The format of the input to the Guardrails AI API. Defaults to 'llmOutput'.", required: false, shape: Shape::Choice(&["inputs", "llmOutput"]), default: Initial::Text("llmOutput"), extra: Extra::NONE },
] };
