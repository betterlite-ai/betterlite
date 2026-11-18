use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_models::decisions::Json;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Frame {
    Text { text: String },
    Binary { text: String },
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct ProviderDocument {
    pub(crate) requires_session_configuration: bool,
    pub(crate) setup: Vec<Json>,
    pub(crate) content: Vec<Json>,
    pub(crate) transforms: Vec<Vec<Frame>>,
    pub(crate) fail_transform: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct GuardrailDocument {
    pub(crate) custom: bool,
    pub(crate) runs_for: Vec<GuardrailEventHooks>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Operation {
    pub(crate) kind: String,
    pub(crate) text: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Input {
    pub(crate) model: String,
    pub(crate) force_transcription_model: Option<String>,
    pub(crate) client_wants_beta: bool,
    pub(crate) backend_uses_beta_protocol: Option<bool>,
    pub(crate) provider: Option<ProviderDocument>,
    pub(crate) guardrails: Vec<GuardrailDocument>,
    pub(crate) backend_failures: Vec<usize>,
    pub(crate) client_failures: Vec<usize>,
    pub(crate) operations: Vec<Operation>,
}
