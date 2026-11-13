use betterlite_core::cost::image_dispatch::error::ImageDispatchError;
use betterlite_core::cost::image_dispatch::options::{
    OptionalParams, SizePattern, call_type_has_image_response, requested_image_param,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
enum Call {
    Size {
        optional_params: Option<OptionalParams>,
    },
    Param {
        key: String,
        optional_params: Option<OptionalParams>,
    },
    CallType {
        call_type: String,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let pattern = SizePattern::new().map_err(pattern_failure)?;
        let values = input
            .calls
            .iter()
            .map(|call| run_call(&pattern, call))
            .collect::<Vec<Value>>();
        Ok(Value::Array(values))
    }
}

fn pattern_failure(error: ImageDispatchError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn run_call(pattern: &SizePattern, call: &Call) -> Value {
    match call {
        Call::Size { optional_params } => json!(pattern.requested_size(optional_params.as_ref())),
        Call::Param {
            key,
            optional_params,
        } => json!(requested_image_param(optional_params.as_ref(), key)),
        Call::CallType { call_type } => json!(call_type_has_image_response(call_type)),
    }
}
