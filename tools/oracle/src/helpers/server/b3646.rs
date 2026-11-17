use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::pass_through_logging::transcribe_logging::{
    is_priced_job_start, operation_from_target,
};
use http::{HeaderMap, HeaderValue};
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    target: Option<String>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut headers = HeaderMap::new();
        if let Some(target) = &input.target {
            let value = target.parse::<HeaderValue>().or_value_error()?;
            headers.insert("x-amz-target", value);
        }
        Ok(json!({
            "operation": operation_from_target(input.target.as_deref()),
            "priced": is_priced_job_start(&headers),
        }))
    }
}
