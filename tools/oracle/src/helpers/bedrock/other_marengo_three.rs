use betterlite_bedrock::marengo_three::error::Marengo3Error;
use betterlite_bedrock::marengo_three::params::RawParams;
use betterlite_bedrock::marengo_three::{build_marengo_3_request, is_marengo_3_model};
use serde::Deserialize;
use serde_json::json;
use thiserror::Error;

use crate::helpers::{Helper, HelperError, HelperResult};

mod tests;

pub(crate) struct Op;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(crate) struct Call {
    model: Option<String>,
    input: String,
    inference_params: RawParams,
    drop_params: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Marengo3ModelDetection(Call),
    NamedMediaSources(Call),
    RequiredBucketOwner(Call),
    TextVersusTimedOptions(Call),
    #[serde(rename = "reject_27_only_options")]
    Reject27OnlyOptions(Call),
    DropPolicy(Call),
    RequireInputMedia(Call),
}

#[derive(Debug, Error)]
enum Failure {
    #[error(transparent)]
    Marengo(#[from] Marengo3Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let (Input::Marengo3ModelDetection(call)
        | Input::NamedMediaSources(call)
        | Input::RequiredBucketOwner(call)
        | Input::TextVersusTimedOptions(call)
        | Input::Reject27OnlyOptions(call)
        | Input::DropPolicy(call)
        | Input::RequireInputMedia(call)) = input;
        let request =
            build_marengo_3_request(&call.input, &call.inference_params, call.drop_params)
                .map_err(Failure::from)?;
        Ok(json!({
            "detected": is_marengo_3_model(call.model.as_deref()),
            "request": serde_json::to_value(&request).map_err(Failure::from)?,
        }))
    }
}

impl From<Failure> for Box<HelperError> {
    fn from(failure: Failure) -> Self {
        let (class, status) = match &failure {
            Failure::Marengo(Marengo3Error::Provider { status, .. }) => {
                ("BedrockError", Some(*status))
            }
            Failure::Marengo(Marengo3Error::Header(_)) => ("HeaderError", None),
            Failure::Json(_) => ("TypeError", None),
        };
        Self::new(HelperError {
            class: class.to_owned(),
            status,
            error_type: None,
            param: None,
            code: None,
            message: failure.to_string(),
        })
    }
}
