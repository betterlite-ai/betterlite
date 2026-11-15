use serde_json::json;

use crate::guardrails::provider_forms::spec::{FormExtra, FormField, FormModel, FormShape};

#[must_use]
pub(crate) fn tables() -> Vec<(&'static str, FormModel)> {
    vec![
        ("block_code_execution", block_code_execution()),
        ("qostodian_nexus", qostodian_nexus()),
        ("hiddenlayer", hiddenlayer()),
        ("javelin", javelin()),
        (
            "mcp_end_user_permission",
            FormModel {
                ui_friendly_name: Some("MCP End User Permission"),
                fields: Vec::new(),
            },
        ),
        ("openai_moderation", openai_moderation()),
        ("xecguard", xecguard()),
        ("azure/prompt_shield", azure_prompt_shield()),
        ("azure/text_moderations", azure_text_moderations()),
        ("alice", alice()),
    ]
}

#[rustfmt::skip]
fn block_code_execution() -> FormModel {
    FormModel { ui_friendly_name: Some("Block Code Execution"), fields: vec![
        FormField { name: "action", description: Some("'block' raises an error; 'mask' replaces the code block with a placeholder."), required: false, shape: FormShape::Choice(vec![json!("block"), json!("mask")]), default: Some(json!("block")), extra: FormExtra::NONE },
        FormField { name: "blocked_languages", description: Some("Language tags to block (e.g. python, javascript, bash). Empty or None = block all fenced code blocks."), required: false, shape: FormShape::List { options: Vec::new() }, default: None, extra: FormExtra { ui_type: Some("multiselect".to_owned()), type_override: None, options: Some(vec![json!("python"), json!("javascript"), json!("typescript"), json!("bash"), json!("ruby"), json!("go"), json!("java"), json!("csharp"), json!("php"), json!("c"), json!("cpp"), json!("rust"), json!("sql")]), min: None, max: None, step: None, default_value: None } },
        FormField { name: "confidence_threshold", description: Some("Only block or mask when detection confidence >= this value; below threshold, allow or log_only."), required: false, shape: FormShape::Number, default: Some(json!(0.5_f64)), extra: FormExtra { ui_type: Some("percentage".to_owned()), type_override: None, options: None, min: Some(json!(0.0_f64)), max: Some(json!(1.0_f64)), step: Some(json!(0.1_f64)), default_value: Some(json!(0.5_f64)) } },
        FormField { name: "detect_execution_intent", description: Some("When True, block only when user intent is to run/execute; allow when intent is explain/refactor/don't run. Also block text-only execution requests (e.g. 'run `ls`', 'read /etc/passwd')."), required: false, shape: FormShape::Boolean, default: Some(json!(true)), extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
fn qostodian_nexus() -> FormModel {
    FormModel { ui_friendly_name: Some("Qostodian Nexus"), fields: vec![
        FormField { name: "api_base", description: Some("The API base URL for Qostodian Nexus. If not provided, the `QOSTODIAN_NEXUS_API_BASE` environment variable is checked. Defaults to http://nexus:8800."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
fn hiddenlayer() -> FormModel {
    FormModel { ui_friendly_name: Some("Hiddenlayer Guardrail"), fields: vec![
        FormField { name: "api_base", description: Some("The URL of the Hiddenlayer server. If not provided, the `HIDDENLAYER_API_BASE` environment variable is checked or https://api.hiddenlayer.ai is used."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "api_id", description: Some("The Hiddenlayer API Id for the Hiddenlayer API. If not provided, the `HIDDENLAYER_CLIENT_ID` environment variable is checked or https://api.hiddenlayer.ai is used."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "api_key", description: Some("The Hiddenlayer Secret Key for the Hiddenlayer API.. If not provided, the `HIDDENLAYER_CLIENT_SECRET` environment variable is checked."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "version", description: Some("Hiddenlayer guardrail version to use."), required: false, shape: FormShape::Number, default: Some(json!(2_i64)), extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
fn javelin() -> FormModel {
    FormModel { ui_friendly_name: Some("Javelin Guardrails"), fields: vec![
        FormField { name: "api_version", description: Some("API version for Javelin service"), required: false, shape: FormShape::Text, default: Some(json!("v1")), extra: FormExtra::NONE },
        FormField { name: "application", description: Some("Application name for Javelin service"), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "config", description: Some("Configuration parameters for Javelin service"), required: false, shape: FormShape::RawObject, default: None, extra: FormExtra::NONE },
        FormField { name: "guard_name", description: Some("Name of the Javelin guard to use"), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "metadata", description: Some("Additional metadata to send with requests"), required: false, shape: FormShape::RawObject, default: None, extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
fn openai_moderation() -> FormModel {
    FormModel { ui_friendly_name: Some("OpenAI Moderation"), fields: vec![
        FormField { name: "api_base", description: Some("OpenAI API base URL. Defaults to 'https://api.openai.com/v1'."), required: false, shape: FormShape::Text, default: Some(json!("https://api.openai.com/v1")), extra: FormExtra::NONE },
        FormField { name: "api_key", description: Some("OpenAI API key. Can also be set via OPENAI_API_KEY environment variable."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "model", description: Some("The OpenAI moderation model to use. 'omni-moderation-latest' supports more categorization options and multi-modal inputs. Defaults to 'omni-moderation-latest'."), required: false, shape: FormShape::Choice(vec![json!("omni-moderation-latest"), json!("text-moderation-latest")]), default: Some(json!("omni-moderation-latest")), extra: FormExtra::NONE },
        FormField { name: "streaming_end_of_stream_only", description: Some("If False (default), moderation runs on sampled chunks during the stream at the cadence set by streaming_sampling_rate, and an in-flight violation stops further chunks from streaming. If True, moderation runs once at end of stream over the assembled response — lower cost and latency, but flagged content has already streamed to the client before the terminal block."), required: false, shape: FormShape::Boolean, default: Some(json!(false)), extra: FormExtra::NONE },
        FormField { name: "streaming_sampling_rate", description: Some("When streaming_end_of_stream_only is False, moderation runs every Nth streamed chunk. Ignored when streaming_end_of_stream_only is True."), required: false, shape: FormShape::Number, default: Some(json!(5_i64)), extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
fn xecguard() -> FormModel {
    FormModel { ui_friendly_name: Some("XecGuard"), fields: vec![
        FormField { name: "api_base", description: Some("XecGuard API base URL. Defaults to https://api-xecguard.cycraft.ai. Falls back to the XECGUARD_API_BASE env var."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "api_key", description: Some("Service Token for XecGuard (prefix 'xgs_'). If not provided, the XECGUARD_API_KEY environment variable is used."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "block_on_error", description: Some("Whether to block requests when the XecGuard API is unreachable. Defaults to true (fail-closed). Falls back to the XECGUARD_BLOCK_ON_ERROR env var."), required: false, shape: FormShape::Boolean, default: None, extra: FormExtra::NONE },
        FormField { name: "grounding_strictness", description: Some("Strictness level for XecGuard context-grounding validation. 'BALANCED' (default) treats INCOMPLETE answers as SAFE; 'STRICT' flags them as UNSAFE. Grounding only runs in post_call when `metadata.xecguard_grounding_documents` is provided."), required: false, shape: FormShape::Choice(vec![json!("BALANCED"), json!("STRICT")]), default: None, extra: FormExtra::NONE },
        FormField { name: "policy_names", description: Some("XecGuard policies to apply on each scan. Select one or more of the built-in default policies; if none are selected, the guardrail defaults to System Prompt Enforcement + Harmful Content Protection."), required: false, shape: FormShape::List { options: Vec::new() }, default: None, extra: FormExtra { ui_type: Some("multiselect".to_owned()), type_override: None, options: Some(vec![json!("Default_Policy_SystemPromptEnforcement"), json!("Default_Policy_GeneralPromptAttackProtection"), json!("Default_Policy_ContentBiasProtection"), json!("Default_Policy_HarmfulContentProtection"), json!("Default_Policy_SkillsProtection"), json!("Default_Policy_PIISensitiveDataProtection")]), min: None, max: None, step: None, default_value: None } },
        FormField { name: "xecguard_model", description: Some("XecGuard scanning model identifier. Defaults to 'xecguard_v2'."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
fn azure_prompt_shield() -> FormModel {
    FormModel { ui_friendly_name: Some("Azure Content Safety Prompt Shield"), fields: vec![
        FormField { name: "api_base", description: Some("Base URL for the Azure Content Safety Prompt Shield guardrail"), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "api_key", description: Some("API key for the Azure Content Safety Prompt Shield guardrail"), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "api_version", description: Some("API version for the Azure Content Safety Prompt Shield guardrail. Default is 2024-09-01"), required: false, shape: FormShape::Text, default: Some(json!("2024-09-01")), extra: FormExtra::NONE },
        FormField { name: "cost_tier", description: Some("Billing tier of the Azure Content Safety resource: 'free' reports usage with cost 0, 'paid' prices usage with price_per_1000_text_records (required for 'paid'). Omit to track usage without a cost estimate"), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "price_per_1000_text_records", description: Some("USD price per 1,000 text records (1 text record = 1,000 characters) used to estimate Prompt Shield cost. 0 marks the free tier; omit to track usage without a cost estimate"), required: false, shape: FormShape::Number, default: None, extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
fn azure_text_moderations() -> FormModel {
    FormModel { ui_friendly_name: Some("Azure Content Safety Text Moderation"), fields: vec![
        FormField { name: "api_base", description: Some("Base URL for the Azure Content Safety Prompt Shield guardrail"), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "api_key", description: Some("API key for the Azure Content Safety Prompt Shield guardrail"), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "api_version", description: Some("API version for the Azure Content Safety Prompt Shield guardrail. Default is 2024-09-01"), required: false, shape: FormShape::Text, default: Some(json!("2024-09-01")), extra: FormExtra::NONE },
        FormField { name: "optional_params", description: Some("Optional parameters for the Azure Content Safety Text Moderation guardrail"), required: true, shape: FormShape::Nested(Box::new(azure_text_moderations_optional_params_model())), default: None, extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
fn azure_text_moderations_optional_params_model() -> FormModel {
    FormModel { ui_friendly_name: None, fields: vec![
        FormField { name: "blocklistNames", description: Some("Blocklist names to scan for the Azure Content Safety Text Moderation guardrail. Learn more - https://learn.microsoft.com/en-us/azure/ai-services/content-safety/quickstart-text"), required: false, shape: FormShape::List { options: Vec::new() }, default: None, extra: FormExtra::NONE },
        FormField { name: "categories", description: Some("Categories to scan for the Azure Content Safety Text Moderation guardrail. See list of categories - https://learn.microsoft.com/en-us/azure/ai-services/content-safety/concepts/harm-categories?tabs=warning"), required: false, shape: FormShape::List { options: vec![json!("Hate"), json!("SelfHarm"), json!("Sexual"), json!("Violence")] }, default: None, extra: FormExtra::NONE },
        FormField { name: "haltOnBlocklistHit", description: Some("Whether to halt the request if a blocklist hit is detected"), required: false, shape: FormShape::Boolean, default: None, extra: FormExtra::NONE },
        FormField { name: "outputType", description: Some("Output type for the Azure Content Safety Text Moderation guardrail. Learn more - https://learn.microsoft.com/en-us/azure/ai-services/content-safety/quickstart-text"), required: false, shape: FormShape::Choice(vec![json!("FourSeverityLevels"), json!("EightSeverityLevels")]), default: None, extra: FormExtra::NONE },
        FormField { name: "severity_threshold", description: Some("Severity threshold for the Azure Content Safety Text Moderation guardrail across all categories"), required: false, shape: FormShape::Number, default: None, extra: FormExtra::NONE },
        FormField { name: "severity_threshold_by_category", description: Some("Severity threshold by category for the Azure Content Safety Text Moderation guardrail. See list of categories - https://learn.microsoft.com/en-us/azure/ai-services/content-safety/concepts/harm-categories?tabs=warning"), required: false, shape: FormShape::Mapping { keys: vec![json!("Hate"), json!("SelfHarm"), json!("Sexual"), json!("Violence")], value: Box::new(FormShape::Number) }, default: None, extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
fn alice() -> FormModel {
    FormModel { ui_friendly_name: Some("Alice"), fields: vec![
        FormField { name: "api_base", description: Some("The API base URL for Alice. If not provided, the `ALICE_API_BASE` environment variable is checked, then `https://api.alice.io`."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "api_key", description: Some("The API key for Alice. If not provided, the `ALICE_API_KEY` environment variable is checked."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
    ] }
}
