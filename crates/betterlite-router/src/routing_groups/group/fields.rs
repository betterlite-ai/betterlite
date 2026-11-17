use std::collections::BTreeMap;

use betterlite_models::decisions::Json;

use crate::router_predicates::JsonMap;
use crate::routing_groups::error::RoutingGroupError;
use crate::routing_groups::group::MAX_MODEL_PRIORITY;

fn text_field(map: &JsonMap, field: &'static str) -> Result<String, RoutingGroupError> {
    match map.get(field) {
        Some(Json::Text(text)) => Ok(text.clone()),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(RoutingGroupError::Field {
                field,
                expected: "a string",
            })
        }
    }
}

fn models_field(map: &JsonMap) -> Result<Vec<String>, RoutingGroupError> {
    let Some(Json::List(entries)) = map.get("models") else {
        return Err(RoutingGroupError::Field {
            field: "models",
            expected: "a list of strings",
        });
    };
    entries
        .iter()
        .map(|entry| match entry {
            Json::Text(text) => Ok(text.clone()),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                Err(RoutingGroupError::Field {
                    field: "models",
                    expected: "a list of strings",
                })
            }
        })
        .collect()
}

fn args_field(map: &JsonMap) -> Result<Option<JsonMap>, RoutingGroupError> {
    match map.get("routing_strategy_args") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(args)) => Ok(Some(args.clone())),
        Some(_) => Err(RoutingGroupError::Field {
            field: "routing_strategy_args",
            expected: "an object",
        }),
    }
}

fn priorities_field(map: &JsonMap) -> Result<Option<BTreeMap<String, u64>>, RoutingGroupError> {
    let raw = match map.get("model_priorities") {
        None | Some(Json::Null) => return Ok(None),
        Some(Json::Object(priorities)) => priorities,
        Some(_) => {
            return Err(RoutingGroupError::Field {
                field: "model_priorities",
                expected: "an object of integers",
            });
        }
    };
    let mut priorities = BTreeMap::new();
    for (model, value) in raw {
        let Json::Number(number) = value else {
            return Err(priority_value(model));
        };
        let Some(priority) = number.as_u64() else {
            return Err(priority_value(model));
        };
        if !(1..=MAX_MODEL_PRIORITY).contains(&priority) {
            return Err(priority_value(model));
        }
        priorities.insert(model.clone(), priority);
    }
    Ok(Some(priorities))
}

fn priority_value(model: &str) -> RoutingGroupError {
    RoutingGroupError::PriorityValue {
        model: model.to_owned(),
    }
}
