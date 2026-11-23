use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::Failure;
use super::result::kinds::SdkResult;
use betterlite_models::decisions::Json;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::future::Future;
mod config;
mod native;
mod ports;
mod support;
// One decorated responses() call: each run is the named arguments and kwargs the case wrote
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
        send(self, api_base, env)
    }
}
async fn send(
    call: &Call,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    Ok(SdkResult::ResponsesCreate {
        value: Box::new(native::execute(call, api_base, env).await?),
    })
}
