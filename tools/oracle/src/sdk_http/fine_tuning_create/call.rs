use super::native;
use crate::sdk_http::OracleLimits;
use crate::sdk_http::call_kind::CallKind;
use crate::sdk_http::error::SdkHttpError;
use crate::sdk_http::result::kinds::SdkResult;
use betterlite_core::api::fine_tuning_create::connection::CreateParams;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::future::Future;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub runs: Vec<Run>,
}
// one create_fine_tuning_job call: the named arguments and the kwargs GenericLiteLLMParams reads; the case env feeds the settings chains
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Run {
    pub provider: String,
    #[serde(default)]
    pub is_async: bool,
    pub model: String,
    pub training_file: String,
    #[serde(default)]
    pub hyperparameters: Option<Value>,
    #[serde(default)]
    pub suffix: Option<String>,
    #[serde(default)]
    pub validation_file: Option<String>,
    #[serde(default)]
    pub integrations: Option<Vec<String>>,
    #[serde(default)]
    pub seed: Option<i64>,
    // the public arguments the source accepts and never forwards
    #[serde(default)]
    pub extra_headers: Option<IndexMap<String, String>>,
    #[serde(default)]
    pub extra_body: Option<Value>,
    #[serde(default)]
    pub params: CreateParams,
}
impl CallKind for Call {
    fn validate(&self, _limits: &OracleLimits) -> Result<(), SdkHttpError> {
        if self.runs.is_empty() {
            return Err(SdkHttpError::Invalid(
                "a fine-tuning create call needs at least one run".to_owned(),
            ));
        }
        Ok(())
    }
    fn send(
        &self,
        api_base: &str,
        env: &BTreeMap<String, String>,
    ) -> impl Future<Output = Result<SdkResult, SdkHttpError>> {
        let runs = self.runs.clone();
        let request_base = api_base.to_owned();
        let request_env = env.clone();
        async move { native::run(&runs, &request_base, &request_env).await }
    }
}
