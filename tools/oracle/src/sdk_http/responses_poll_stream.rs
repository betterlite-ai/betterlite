use std::collections::BTreeMap;
use std::future::Future;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::OracleLimits;
use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::kinds::SdkResult;

mod capture;
mod clock;
mod native;
mod process;
mod request;

// background_streaming_task's inputs: the admitted body, the record seeded in the cache and the clock
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub(crate) api_key: String,
    pub(crate) polling_id: String,
    pub(crate) data: IndexMap<String, Json>,
    pub(crate) seed: Json,
    pub(crate) clock_millis: Vec<u64>,
    #[serde(default)]
    pub(crate) cancel_after_write: Option<usize>,
}

// every cache write the worker made, in order
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub(crate) writes: Vec<Json>,
}

impl CallKind for Call {
    fn validate(&self, _limits: &OracleLimits) -> Result<(), SdkHttpError> {
        if self.clock_millis.is_empty() {
            return Err(SdkHttpError::Invalid(
                "the poll stream case scripts no clock readings".to_owned(),
            ));
        }
        if !matches!(self.seed, Json::Object(_)) {
            return Err(SdkHttpError::Invalid(
                "the poll stream seed must be a response object".to_owned(),
            ));
        }
        if !matches!(self.data.get("model"), Some(Json::Text(_))) {
            return Err(SdkHttpError::Invalid(
                "the poll stream data needs a string model".to_owned(),
            ));
        }
        if matches!(self.cancel_after_write, Some(0)) {
            return Err(SdkHttpError::Invalid(
                "cancel_after_write counts the writes from one".to_owned(),
            ));
        }
        Ok(())
    }

    fn send(
        &self,
        api_base: &str,
        env: &BTreeMap<String, String>,
    ) -> impl Future<Output = Result<SdkResult, SdkHttpError>> {
        native::execute(self, api_base, env)
    }
}
