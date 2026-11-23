use std::collections::BTreeMap;
use std::future::Future;

use serde::{Deserialize, Serialize};

use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::image_generation_call;
use super::result::kinds::SdkResult;

mod native;
mod operations;
mod report;
mod sends;

// the mock origin a case's environment and call settings write as {mock}
pub(super) const MOCK: &str = "{mock}";

// Which litellm entry the driver calls: the sync function, or the async wrapper this task owns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncMode {
    Sync,
    #[serde(rename = "async")]
    Asynchronous,
}

// B998's operation encoding, driven through the entry point under test.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub sync_mode: SyncMode,
    pub call: image_generation_call::Call,
}

// the response or the SDK exception the call answers, exactly as B998's call reports it
pub type Outcome = image_generation_call::Outcome;

impl CallKind for Call {
    fn send(
        &self,
        api_base: &str,
        env: &BTreeMap<String, String>,
    ) -> impl Future<Output = Result<SdkResult, SdkHttpError>> {
        native::execute(self, api_base, env)
    }
}
