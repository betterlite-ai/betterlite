use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::deployment_checks::{
    DeploymentCheckError, check_access_windows, check_capability_limits,
    check_complexity_placement, check_max_agentic_loops, pin_complexity_model_id,
};
use jiff::tz::TimeZoneDatabase;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    MaxAgenticLoops {
        model: JsonObject,
    },
    ComplexityPlacement {
        model: JsonObject,
    },
    AccessWindows {
        model: JsonObject,
    },
    CapabilityLimits {
        model_list: Vec<JsonObject>,
        limit: Option<usize>,
    },
    PinModelId {
        model: JsonObject,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::MaxAgenticLoops { model } => check_output(model, check_max_agentic_loops(model)),
            Input::ComplexityPlacement { model } => {
                check_output(model, check_complexity_placement(model))
            }
            Input::AccessWindows { model } => {
                let database = TimeZoneDatabase::from_env();
                check_output(model, check_access_windows(model, &database))
            }
            Input::CapabilityLimits { model_list, limit } => {
                limits_output(check_capability_limits(model_list, *limit))
            }
            Input::PinModelId { model } => pinned_output(model),
        }
    }
}

fn check_output(model: &JsonObject, result: Result<(), DeploymentCheckError>) -> HelperResult {
    let name = model.get("model_name").cloned().unwrap_or(Json::Null);
    match result {
        Ok(()) => Ok(serde_json::json!({"refused": false, "model_name": name, "message": null})),
        Err(error) => Ok(
            serde_json::json!({"refused": true, "model_name": name, "message": error.to_string()}),
        ),
    }
}

fn limits_output(result: Result<(), DeploymentCheckError>) -> HelperResult {
    match result {
        Ok(()) => Ok(serde_json::json!({"refused": false, "message": null})),
        Err(error) => Ok(serde_json::json!({"refused": true, "message": error.to_string()})),
    }
}

fn pinned_output(model: &JsonObject) -> HelperResult {
    let mut pinned = model.clone();
    pin_complexity_model_id(&mut pinned);
    serde_json::to_value(&pinned).or_value_error()
}

mod tests;
