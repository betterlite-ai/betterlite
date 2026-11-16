use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::microsoft_purview::purview_dlp_hooks::Data;
use serde::Deserialize;

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Function {
    PreCall,
    PostSuccess,
    Streaming,
    LoggingSync,
    LoggingDeferred,
    LoggingAsync,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GuardrailConfig {
    pub(crate) guardrail_name: String,
    pub(crate) tenant_id: String,
    pub(crate) client_id: String,
    pub(crate) client_secret: String,
    pub(crate) purview_app_name: Option<String>,
    pub(crate) user_id_field: Option<String>,
    pub(crate) timeout_seconds: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Identity {
    pub(crate) user_id: Option<String>,
    pub(crate) end_user_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Reply {
    pub(crate) status: u16,
    pub(crate) body: GuardrailValue,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) function: Function,
    pub(crate) guardrail: GuardrailConfig,
    pub(crate) replies: Vec<Reply>,
    pub(crate) identity: Option<Identity>,
    pub(crate) call_type: Option<String>,
    pub(crate) data: Data,
    pub(crate) result: Option<GuardrailValue>,
    pub(crate) chunks: Option<Vec<GuardrailValue>>,
}
