use betterlite_openai::fine_tuning::job::FineTuneCreate;
use betterlite_vertex::provider_metadata::timestamp::LocalTimeContext;
use betterlite_vertex::vertex_fine_tuning::records::request::{
    OriginalHyperparameters, to_vertex_request,
};
use betterlite_vertex::vertex_fine_tuning::records::response::decode_vertex_tuning_job;
use jiff::tz::TimeZone;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::HelperError;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Request { calls: Vec<RequestCall> },
    Response { calls: Vec<ResponseCall> },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequestCall {
    data: FineTuneCreate,
    original_hyperparameters: OriginalHyperparameters,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResponseCall {
    zone: String,
    response: Value,
}

pub(super) fn run(input: &Input) -> Result<Value, Box<HelperError>> {
    match input {
        Input::Request { calls } => Ok(json!({
            "calls": calls.iter().map(request_outcome).collect::<Vec<_>>(),
        })),
        Input::Response { calls } => {
            let outcomes = calls
                .iter()
                .map(response_outcome)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(json!({"calls": outcomes}))
        }
    }
}

fn request_outcome(call: &RequestCall) -> Value {
    match to_vertex_request(&call.data, &call.original_hyperparameters) {
        Ok(job) => json!({"ok": true, "job": job}),
        Err(_error) => json!({"ok": false}),
    }
}

fn response_outcome(call: &ResponseCall) -> Result<Value, Box<HelperError>> {
    let zone = TimeZone::get(&call.zone).map_err(|error| invalid_input(format!("{error}")))?;
    let local = LocalTimeContext { zone };
    let bytes =
        serde_json::to_vec(&call.response).map_err(|error| invalid_input(error.to_string()))?;
    Ok(match decode_vertex_tuning_job(&bytes, &local) {
        Ok(job) => json!({"ok": true, "job": job}),
        Err(_error) => json!({"ok": false}),
    })
}

fn invalid_input(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
