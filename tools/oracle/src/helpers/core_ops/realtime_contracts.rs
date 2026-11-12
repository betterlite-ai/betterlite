mod body;
mod close_helpers;
mod hooks;
mod records;
use crate::helpers::{Helper, HelperError, HelperResult};
use serde::Deserialize;
#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Function {
    Records(records::RecordsInput),
    Hooks(hooks::HooksInput),
    HttpUrls(hooks::HttpUrlsInput),
    CloseHelpers(close_helpers::CloseHelpersInput),
    CachedBodyAndFrames(body::CachedBodyInput),
}

pub(crate) struct Op;
impl Helper for Op {
    type Input = Function;
    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Function::Records(fields) => records::records(fields),
            Function::Hooks(fields) => Ok(hooks::hooks(fields)),
            Function::HttpUrls(fields) => Ok(hooks::http_urls(fields)),
            Function::CloseHelpers(fields) => close_helpers::close_helpers(fields),
            Function::CachedBodyAndFrames(fields) => body::cached_body_and_frames(fields),
        }
    }
}

pub(super) fn failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "RealtimeContractError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
