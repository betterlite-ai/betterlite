mod fixture;
mod view;

mod tests;

use std::collections::BTreeMap;

use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::pass_through_logging::tinyfish::is_allowed_tinyfish_endpoint;
use betterlite_server::pass_through_logging::tinyfish::payload::build_logging_payload;
use betterlite_server::pass_through_logging::tinyfish::records::parse_run;
use betterlite_server::pass_through_logging::tinyfish::settings::{
    API_BASE_VARIABLE, COST_VARIABLE, api_base_from_text, cost_per_step_from_text,
};
use betterlite_server::pass_through_logging::tinyfish::sse::run_id_from_sse_frames;
use serde::Deserialize;
use serde_json::json;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    function: Function,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    #[serde(default)]
    method: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    run: Option<Json>,
    #[serde(default)]
    frames: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Function {
    RunBilling,
    Endpoint,
    ApiBase,
    CostPerStep,
    SseRunId,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input.function {
            Function::RunBilling => billing(input),
            Function::Endpoint => Ok(json!(is_allowed_tinyfish_endpoint(
                &input.method,
                &input.path
            ))),
            Function::ApiBase => Ok(json!(api_base_from_text(
                input.environment.get(API_BASE_VARIABLE).map(String::as_str)
            ))),
            Function::CostPerStep => Ok(json!(
                cost_per_step_from_text(input.environment.get(COST_VARIABLE).map(String::as_str))
                    .normalize()
                    .to_string()
            )),
            Function::SseRunId => Ok(json!(run_id_from_sse_frames(&input.frames))),
        }
    }
}

fn billing(input: &Input) -> HelperResult {
    let logging_obj = fixture::logging_object()?;
    let submitted = input.run.as_ref().and_then(parse_run);
    let kwargs = PassThroughLoggingKwargs::default();
    let rate = cost_per_step_from_text(input.environment.get(COST_VARIABLE).map(String::as_str));
    drop(
        build_logging_payload(&logging_obj, submitted.as_ref(), "", &kwargs, rate)
            .or_value_error()?,
    );
    view::billing_view(&logging_obj)
}
