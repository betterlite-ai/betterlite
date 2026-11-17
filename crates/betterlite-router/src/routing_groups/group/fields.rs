use std::collections::BTreeMap;

use betterlite_models::decisions::Json;

use crate::router_predicates::JsonMap;
use crate::routing_groups::error::RoutingGroupError;
use crate::routing_groups::group::MAX_MODEL_PRIORITY;

fn field<'map>(
    map: &'map JsonMap,
    field: &'static str,
    expected: &'static str,
) -> Result<&'map Json, RoutingGroupError> {
    map.get(field)
        .ok_or(RoutingGroupError::Field { field, expected })
}

pub(super) fn text_field(map: &JsonMap, key: &'static str) -> Result<String, RoutingGroupError> {
    let Json::Text(text) = field(map, key, "a string")? else {
        return Err(RoutingGroupError::Field {
            field: key,
            expected: "a string",
        });
    };
    Ok(text.clone())
}

const fn models_shape() -> RoutingGroupError {
    RoutingGroupError::Field {
        field: "models",
        expected: "a list of strings",
    }
}

pub(super) fn models_field(map: &JsonMap) -> Result<Vec<String>, RoutingGroupError> {
    let Json::List(entries) = field(map, "models", "a list of strings")? else {
        return Err(models_shape());
    };
    let mut models = Vec::with_capacity(entries.len());
    for entry in entries {
        let Json::Text(model) = entry else {
            return Err(models_shape());
        };
        models.push(model.clone());
    }
    Ok(models)
}

pub(super) fn args_field(map: &JsonMap) -> Result<Option<JsonMap>, RoutingGroupError> {
    match map.get("routing_strategy_args") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(args)) => Ok(Some(args.clone())),
        Some(_) => Err(RoutingGroupError::Field {
            field: "routing_strategy_args",
            expected: "an object",
        }),
    }
}

pub(super) fn priorities_field(
    map: &JsonMap,
) -> Result<Option<BTreeMap<String, u64>>, RoutingGroupError> {
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
