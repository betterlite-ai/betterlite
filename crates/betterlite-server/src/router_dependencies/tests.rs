#![cfg(test)]

use std::collections::BTreeSet;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::router_dependencies::StrategyRouterDependency;

mod router_dependencies_behaviors;
mod router_dependencies_boundaries;

// fixtures are JSON text so a test shows the bytes a caller sends
pub(crate) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the fixture is JSON")
}

pub(crate) fn params(text: &str) -> JsonObject {
    match json(text) {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("the fixture is a JSON object")
        }
    }
}

pub(crate) fn router_config(params: &JsonObject) -> Option<&JsonObject> {
    match params.get("complexity_router_config") {
        Some(Json::Object(fields)) => Some(fields),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => None,
    }
}

pub(crate) fn names(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

// the list a test compares against, as the model name and role a caller reads
pub(crate) fn listed(dependencies: &[StrategyRouterDependency]) -> Vec<(&str, &'static str)> {
    dependencies
        .iter()
        .map(|dependency| (dependency.model_name.as_str(), dependency.role.name()))
        .collect()
}
