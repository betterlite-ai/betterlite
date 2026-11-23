use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::Failure;
use super::result::kinds::SdkResult;
use betterlite_models::decisions::Json;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::future::Future;
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
    fn send(
        &self,
        api_base: &str,
        env: &BTreeMap<String, String>,
    ) -> impl Future<Output = Result<SdkResult, SdkHttpError>> {
        async move {
            Ok(SdkResult::ResponsesCreate {
                value: Box::new(native::execute(self, api_base, env).await?),
            })
        }
    }
}
