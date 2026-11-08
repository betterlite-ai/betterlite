use betterlite_models::fetch_policy::revision::git_blob_id;
use serde_json::json;

use crate::helpers::{HelperError, HelperResult};

use input::{Input, Operation};

mod input;
mod run;
mod scripting;

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        match input.operation {
            Operation::RawBlobRevision => Ok(json!({
                "revision": git_blob_id(input.body.as_bytes()),
            })),
            Operation::ValidateMap => run::validate(input),
            Operation::ClassifyResponse => Ok(run::classify(input)),
            Operation::ParseRetryAfter => Ok(run::parse_retry_after(input)),
            Operation::RetryWait => run::retry_wait(input),
            Operation::NextRetryWait => run::next_retry_wait(input),
            Operation::FetchWithRetry => run::fetch(input),
        }
    }
}

pub(super) fn invalid(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
