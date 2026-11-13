use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::model_registration::merge::{
    convert_stringified_numbers, update_dictionary,
};
use serde::Deserialize;

use crate::helpers::HelperResult;
use project::project;
use registration::{Scenario, register};
use routing::{Deployment, RegionCall, order_filter, region};

mod project;
mod registration;
mod routing;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    UpdateDictionary {
        existing: JsonObject,
        new: JsonObject,
    },
    ConvertStringifiedNumbers {
        value: String,
    },
    RegisterModel(Scenario),
    OrderFilter {
        deployments: Vec<Deployment>,
        target_order: Option<i64>,
    },
    Region(Box<RegionCall>),
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::UpdateDictionary { existing, new } => {
                let mut merged = existing.clone();
                update_dictionary(&mut merged, new);
                Ok(project(&Json::Object(merged)))
            }
            Input::ConvertStringifiedNumbers { value } => {
                Ok(project(&convert_stringified_numbers(value)))
            }
            Input::RegisterModel(scenario) => register(scenario),
            Input::OrderFilter {
                deployments,
                target_order,
            } => Ok(order_filter(deployments, *target_order)),
            Input::Region(found) => region(found),
        }
    }
}
