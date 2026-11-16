use crate::guardrails::form_tables_a::model::{Extra, Field, Initial, Model, Shape};

pub(crate) const TABLES: &[(&str, Model)] = &[
    ("block_code_execution", BLOCK_CODE_EXECUTION),
    ("qostodian_nexus", QOSTODIAN_NEXUS),
    ("hiddenlayer", HIDDENLAYER),
    ("javelin", JAVELIN),
    (
        "mcp_end_user_permission",
        Model {
            name: Some("MCP End User Permission"),
            fields: &[],
        },
    ),
    ("openai_moderation", OPENAI_MODERATION),
    ("xecguard", XECGUARD),
    ("azure/prompt_shield", AZURE_PROMPT_SHIELD),
    ("azure/text_moderations", AZURE_TEXT_MODERATIONS),
    ("alice", ALICE),
];

#[rustfmt::skip]
const BLOCK_CODE_EXECUTION: Model = Model { name: Some("Block Code Execution"), fields: &[
    Field { name: "action", description: "'block' raises an error; 'mask' replaces the code block with a placeholder.", required: false, shape: Shape::Choice(&["block", "mask"]), default: Initial::Text("block"), extra: Extra::NONE },
    Field { name: "blocked_languages", description: "Language tags to block (e.g. python, javascript, bash). Empty or None = block all fenced code blocks.", required: false, shape: Shape::List(&[]), default: Initial::None, extra: Extra { ui_type: Some("multiselect"), options: Some(&["python", "javascript", "typescript", "bash", "ruby", "go", "java", "csharp", "php", "c", "cpp", "rust", "sql"]), ..Extra::NONE } },
    Field { name: "confidence_threshold", description: "Only block or mask when detection confidence >= this value; below threshold, allow or log_only.", required: false, shape: Shape::Number, default: Initial::Float(0.5), extra: Extra { ui_type: Some("percentage"), min: Some(0.0_f64), max: Some(1.0_f64), step: Some(0.1_f64), default_value: Some(0.5_f64), ..Extra::NONE } },
    Field { name: "detect_execution_intent", description: "When True, block only when user intent is to run/execute; allow when intent is explain/refactor/don't run. Also block text-only execution requests (e.g. 'run `ls`', 'read /etc/passwd').", required: false, shape: Shape::Boolean, default: Initial::Bool(true), extra: Extra::NONE },
] };

#[rustfmt::skip]
const QOSTODIAN_NEXUS: Model = Model { name: Some("Qostodian Nexus"), fields: &[
    Field { name: "api_base", description: "The API base URL for Qostodian Nexus. If not provided, the `QOSTODIAN_NEXUS_API_BASE` environment variable is checked. Defaults to http://nexus:8800.", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
] };

#[rustfmt::skip]
const HIDDENLAYER: Model = Model { name: Some("Hiddenlayer Guardrail"), fields: &[
    Field { name: "api_base", description: "The URL of the Hiddenlayer server. If not provided, the `HIDDENLAYER_API_BASE` environment variable is checked or https://api.hiddenlayer.ai is used.", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
    Field { name: "api_id", description: "The Hiddenlayer API Id for the Hiddenlayer API. If not provided, the `HIDDENLAYER_CLIENT_ID` environment variable is checked or https://api.hiddenlayer.ai is used.", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
    Field { name: "api_key", description: "The Hiddenlayer Secret Key for the Hiddenlayer API.. If not provided, the `HIDDENLAYER_CLIENT_SECRET` environment variable is checked.", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
    Field { name: "version", description: "Hiddenlayer guardrail version to use.", required: false, shape: Shape::Number, default: Initial::Int(2), extra: Extra::NONE },
] };

#[rustfmt::skip]
const JAVELIN: Model = Model { name: Some("Javelin Guardrails"), fields: &[
    Field { name: "api_version", description: "API version for Javelin service", required: false, shape: Shape::Text, default: Initial::Text("v1"), extra: Extra::NONE },
    Field { name: "application", description: "Application name for Javelin service", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
    Field { name: "config", description: "Configuration parameters for Javelin service", required: false, shape: Shape::RawObject, default: Initial::None, extra: Extra::NONE },
    Field { name: "guard_name", description: "Name of the Javelin guard to use", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
    Field { name: "metadata", description: "Additional metadata to send with requests", required: false, shape: Shape::RawObject, default: Initial::None, extra: Extra::NONE },
] };

#[rustfmt::skip]
const OPENAI_MODERATION: Model = Model { name: Some("OpenAI Moderation"), fields: &[
    Field { name: "api_base", description: "OpenAI API base URL. Defaults to 'https://api.openai.com/v1'.", required: false, shape: Shape::Text, default: Initial::Text("https://api.openai.com/v1"), extra: Extra::NONE },
    Field { name: "api_key", description: "OpenAI API key. Can also be set via OPENAI_API_KEY environment variable.", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
    Field { name: "model", description: "The OpenAI moderation model to use. 'omni-moderation-latest' supports more categorization options and multi-modal inputs. Defaults to 'omni-moderation-latest'.", required: false, shape: Shape::Choice(&["omni-moderation-latest", "text-moderation-latest"]), default: Initial::Text("omni-moderation-latest"), extra: Extra::NONE },
    Field { name: "streaming_end_of_stream_only", description: "If False (default), moderation runs on sampled chunks during the stream at the cadence set by streaming_sampling_rate, and an in-flight violation stops further chunks from streaming. If True, moderation runs once at end of stream over the assembled response \u{2014} lower cost and latency, but flagged content has already streamed to the client before the terminal block.", required: false, shape: Shape::Boolean, default: Initial::Bool(false), extra: Extra::NONE },
    Field { name: "streaming_sampling_rate", description: "When streaming_end_of_stream_only is False, moderation runs every Nth streamed chunk. Ignored when streaming_end_of_stream_only is True.", required: false, shape: Shape::Number, default: Initial::Int(5), extra: Extra::NONE },
] };

#[rustfmt::skip]
const XECGUARD: Model = Model { name: Some("XecGuard"), fields: &[
    Field { name: "api_base", description: "XecGuard API base URL. Defaults to https://api-xecguard.cycraft.ai. Falls back to the XECGUARD_API_BASE env var.", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
    Field { name: "api_key", description: "Service Token for XecGuard (prefix 'xgs_'). If not provided, the XECGUARD_API_KEY environment variable is used.", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
    Field { name: "block_on_error", description: "Whether to block requests when the XecGuard API is unreachable. Defaults to true (fail-closed). Falls back to the XECGUARD_BLOCK_ON_ERROR env var.", required: false, shape: Shape::Boolean, default: Initial::None, extra: Extra::NONE },
    Field { name: "grounding_strictness", description: "Strictness level for XecGuard context-grounding validation. 'BALANCED' (default) treats INCOMPLETE answers as SAFE; 'STRICT' flags them as UNSAFE. Grounding only runs in post_call when `metadata.xecguard_grounding_documents` is provided.", required: false, shape: Shape::Choice(&["BALANCED", "STRICT"]), default: Initial::None, extra: Extra::NONE },
    Field { name: "policy_names", description: "XecGuard policies to apply on each scan. Select one or more of the built-in default policies; if none are selected, the guardrail defaults to System Prompt Enforcement + Harmful Content Protection.", required: false, shape: Shape::List(&[]), default: Initial::None, extra: Extra { ui_type: Some("multiselect"), options: Some(&["Default_Policy_SystemPromptEnforcement", "Default_Policy_GeneralPromptAttackProtection", "Default_Policy_ContentBiasProtection", "Default_Policy_HarmfulContentProtection", "Default_Policy_SkillsProtection", "Default_Policy_PIISensitiveDataProtection"]), ..Extra::NONE } },
    Field { name: "xecguard_model", description: "XecGuard scanning model identifier. Defaults to 'xecguard_v2'.", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
] };

#[rustfmt::skip]
const AZURE_PROMPT_SHIELD: Model = Model { name: Some("Azure Content Safety Prompt Shield"), fields: &[
    Field { name: "api_base", description: "Base URL for the Azure Content Safety Prompt Shield guardrail", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
    Field { name: "api_key", description: "API key for the Azure Content Safety Prompt Shield guardrail", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
    Field { name: "api_version", description: "API version for the Azure Content Safety Prompt Shield guardrail. Default is 2024-09-01", required: false, shape: Shape::Text, default: Initial::Text("2024-09-01"), extra: Extra::NONE },
    Field { name: "cost_tier", description: "Billing tier of the Azure Content Safety resource: 'free' reports usage with cost 0, 'paid' prices usage with price_per_1000_text_records (required for 'paid'). Omit to track usage without a cost estimate", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
    Field { name: "price_per_1000_text_records", description: "USD price per 1,000 text records (1 text record = 1,000 characters) used to estimate Prompt Shield cost. 0 marks the free tier; omit to track usage without a cost estimate", required: false, shape: Shape::Number, default: Initial::None, extra: Extra::NONE },
] };

#[rustfmt::skip]
const AZURE_TEXT_MODERATIONS: Model = Model { name: Some("Azure Content Safety Text Moderation"), fields: &[
    Field { name: "api_base", description: "Base URL for the Azure Content Safety Prompt Shield guardrail", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
    Field { name: "api_key", description: "API key for the Azure Content Safety Prompt Shield guardrail", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
    Field { name: "api_version", description: "API version for the Azure Content Safety Prompt Shield guardrail. Default is 2024-09-01", required: false, shape: Shape::Text, default: Initial::Text("2024-09-01"), extra: Extra::NONE },
    Field { name: "optional_params", description: "Optional parameters for the Azure Content Safety Text Moderation guardrail", required: true, shape: Shape::Nested(&AZURE_TEXT_OPTIONS), default: Initial::None, extra: Extra::NONE },
] };

#[rustfmt::skip]
const AZURE_TEXT_OPTIONS: Model = Model { name: None, fields: &[
    Field { name: "blocklistNames", description: "Blocklist names to scan for the Azure Content Safety Text Moderation guardrail. Learn more - https://learn.microsoft.com/en-us/azure/ai-services/content-safety/quickstart-text", required: false, shape: Shape::List(&[]), default: Initial::None, extra: Extra::NONE },
    Field { name: "categories", description: "Categories to scan for the Azure Content Safety Text Moderation guardrail. See list of categories - https://learn.microsoft.com/en-us/azure/ai-services/content-safety/concepts/harm-categories?tabs=warning", required: false, shape: Shape::List(&["Hate", "SelfHarm", "Sexual", "Violence"]), default: Initial::None, extra: Extra::NONE },
    Field { name: "haltOnBlocklistHit", description: "Whether to halt the request if a blocklist hit is detected", required: false, shape: Shape::Boolean, default: Initial::None, extra: Extra::NONE },
    Field { name: "outputType", description: "Output type for the Azure Content Safety Text Moderation guardrail. Learn more - https://learn.microsoft.com/en-us/azure/ai-services/content-safety/quickstart-text", required: false, shape: Shape::Choice(&["FourSeverityLevels", "EightSeverityLevels"]), default: Initial::None, extra: Extra::NONE },
    Field { name: "severity_threshold", description: "Severity threshold for the Azure Content Safety Text Moderation guardrail across all categories", required: false, shape: Shape::Number, default: Initial::None, extra: Extra::NONE },
    Field { name: "severity_threshold_by_category", description: "Severity threshold by category for the Azure Content Safety Text Moderation guardrail. See list of categories - https://learn.microsoft.com/en-us/azure/ai-services/content-safety/concepts/harm-categories?tabs=warning", required: false, shape: Shape::Mapping(&["Hate", "SelfHarm", "Sexual", "Violence"], &Shape::Number), default: Initial::None, extra: Extra::NONE },
] };

#[rustfmt::skip]
const ALICE: Model = Model { name: Some("Alice"), fields: &[
    Field { name: "api_base", description: "The API base URL for Alice. If not provided, the `ALICE_API_BASE` environment variable is checked, then `https://api.alice.io`.", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
    Field { name: "api_key", description: "The API key for Alice. If not provided, the `ALICE_API_KEY` environment variable is checked.", required: false, shape: Shape::Text, default: Initial::None, extra: Extra::NONE },
] };
