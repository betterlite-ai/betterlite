use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::Failure;
use super::result::kinds::SdkResult;
use betterlite_models::decisions::Json;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
mod config;
#[path = "responses_create/fixtures/support.rs"]
mod fixtures;
mod native;
mod ports;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub(crate) runs: Vec<Json>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub(crate) runs: Vec<RunOutcome>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunOutcome {
    pub(crate) result: Option<Json>,
    pub(crate) failure: Option<Failure>,
}
impl CallKind for Call {
    async fn send(
        &self,
        api_base: &str,
        env: &BTreeMap<String, String>,
    ) -> Result<SdkResult, SdkHttpError> {
        Ok(SdkResult::ResponsesCreate {
            value: Box::new(native::execute(self, api_base, env).await?),
        })
    }
}
