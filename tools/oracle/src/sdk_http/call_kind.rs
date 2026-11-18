use std::collections::BTreeMap;
use std::future::{self, Future};

use serde::{Deserialize, Serialize};

use super::OracleLimits;
use super::error::SdkHttpError;
use super::result::kinds::SdkResult;

// A pre-registered call kind fills these in its own module, so the shared enums and dispatch never change for it
pub(crate) trait CallKind {
    fn validate(&self, _limits: &OracleLimits) -> Result<(), SdkHttpError> {
        Ok(())
    }

    fn send(
        &self,
        api_base: &str,
        env: &BTreeMap<String, String>,
    ) -> impl Future<Output = Result<SdkResult, SdkHttpError>>;
}

// Stands in for a call kind's call and outcome until the task numbered TASK builds them
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unbuilt<const TASK: u16>;

impl<const TASK: u16> CallKind for Unbuilt<TASK> {
    fn send(
        &self,
        _api_base: &str,
        _env: &BTreeMap<String, String>,
    ) -> impl Future<Output = Result<SdkResult, SdkHttpError>> {
        future::ready(Err(SdkHttpError::NotBuilt { task: TASK }))
    }
}
