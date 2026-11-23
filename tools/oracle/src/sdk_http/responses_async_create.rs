use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::Failure;
use super::result::kinds::SdkResult;
use betterlite_models::decisions::Json;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

mod config;
#[path = "responses_async_create/fixtures/support.rs"]
mod fixtures;
mod native;
#[path = "responses_async_create/fixtures/scripted.rs"]
mod scripted;

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
    pub(crate) hooks: HookObservation,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookObservation {
    pub(crate) checks: u32,
    pub(crate) ran: u32,
    pub(crate) sync_checks: u32,
    pub(crate) calls: Vec<HookCall>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookCall {
    pub(crate) model: String,
    pub(crate) prompt_id: Option<String>,
    pub(crate) prompt_label: Option<String>,
    pub(crate) prompt_version: Option<Json>,
    pub(crate) prompt_variables: Option<Json>,
    pub(crate) messages: Json,
    pub(crate) carry: bool,
    pub(crate) aresponses: bool,
}

impl CallKind for Call {
    async fn send(
        &self,
        api_base: &str,
        env: &BTreeMap<String, String>,
    ) -> Result<SdkResult, SdkHttpError> {
        Ok(SdkResult::ResponsesAsyncCreate {
            value: Box::new(native::execute(self, api_base, env).await?),
        })
    }
}
