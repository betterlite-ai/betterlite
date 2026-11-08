mod tests;

use betterlite_router::weighted_failover::drop_excluded;
use betterlite_sdk::silent_experiment::{silent_kwargs, silent_targets};
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    SilentTargets {
        value: Option<Value>,
    },
    SilentKwargs {
        request: Map<String, Value>,
    },
    DropExcluded {
        deployments: Vec<Value>,
        excluded: Vec<String>,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::SilentTargets { value } => target_list(value.as_ref()),
            Input::SilentKwargs { request } => silent_kwargs(request)
                .map(Value::Object)
                .map_err(|error| helper_error("SilentExperimentError", error.to_string())),
            Input::DropExcluded {
                deployments,
                excluded,
            } => serde_json::to_value(drop_excluded(
                deployments.clone(),
                excluded,
                |deployment: &Value| deployment.get("model_info")?.get("id")?.as_str(),
            ))
            .map_err(|error| helper_error("SerializationError", error.to_string())),
        }
    }
}

fn target_list(value: Option<&Value>) -> HelperResult {
    Ok(Value::Array(
        silent_targets(value)
            .into_iter()
            .map(Value::String)
            .collect(),
    ))
}

fn helper_error(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
