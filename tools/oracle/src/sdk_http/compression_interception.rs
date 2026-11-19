use std::collections::BTreeMap;
use std::future::Future;

use betterlite_core::routes::CallType;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::kinds::SdkResult;

mod run;
mod session;

// The two config sections LiteLLM's initialize_from_proxy_config reads, as the case gives them.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsSource {
    #[serde(default)]
    pub litellm_settings: IndexMap<String, Json>,
    #[serde(default)]
    pub callback_settings: IndexMap<String, Json>,
}

// What the scripted compress returns for one call; the case supplies it instead of the real engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptedResult {
    pub messages: Vec<Json>,
    pub tools: Vec<Json>,
    pub original_tokens: Json,
    pub compressed_tokens: Json,
    pub cache: IndexMap<String, String>,
    #[serde(default)]
    pub skipped_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    #[serde(default)]
    pub settings: Option<SettingsSource>,
    pub request: IndexMap<String, Json>,
    #[serde(default)]
    pub call_type: Option<CallType>,
    pub now: f64,
    #[serde(default)]
    pub lookup: Option<String>,
    #[serde(default)]
    pub compress: Option<ScriptedResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsRecord {
    pub enabled: bool,
    pub compression_trigger: usize,
    pub compression_target: Option<usize>,
    pub embedding_model: Option<String>,
    pub embedding_model_params: Option<IndexMap<String, Json>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompressCallRecord {
    pub model: String,
    pub call_type: CallType,
    pub compression_trigger: usize,
    pub compression_target: Option<usize>,
    pub embedding_model: Option<String>,
    pub embedding_model_params: Option<IndexMap<String, Json>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticRecord {
    pub level: String,
    pub kind: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepOutcome {
    pub settings: SettingsRecord,
    pub skipped: bool,
    pub request: IndexMap<String, Json>,
    pub compress_calls: Vec<CompressCallRecord>,
    pub lookup: Vec<String>,
    pub diagnostics: Vec<DiagnosticRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub steps: Vec<StepOutcome>,
}

impl CallKind for Call {
    fn send(
        &self,
        api_base: &str,
        env: &BTreeMap<String, String>,
    ) -> impl Future<Output = Result<SdkResult, SdkHttpError>> {
        run::send(self, api_base, env)
    }
}
