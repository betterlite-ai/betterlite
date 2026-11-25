use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use serde::{Deserialize, Serialize};

use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::Failure;
use super::result::kinds::SdkResult;

#[path = "response_chat_dispatch/fixtures/answers.rs"]
mod answers;
#[path = "response_chat_dispatch/fixtures/config.rs"]
mod config;
#[path = "response_chat_dispatch/fixtures/execution.rs"]
mod execution;
#[path = "response_chat_dispatch/fixtures/support.rs"]
mod fixtures;
mod native;
#[path = "response_chat_dispatch/fixtures/projection.rs"]
mod projection;

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

// The chat response or drained stream the bridge handed back, or the failure it raised
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunOutcome {
    pub(crate) result: Option<Json>,
    pub(crate) stream: Option<Json>,
    pub(crate) failure: Option<Failure>,
}

impl CallKind for Call {
    async fn send(
        &self,
        api_base: &str,
        env: &BTreeMap<String, String>,
    ) -> Result<SdkResult, SdkHttpError> {
        Ok(SdkResult::ResponseChatDispatch {
            value: Box::new(native::execute(self, api_base, env).await?),
        })
    }
}
