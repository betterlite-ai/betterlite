use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use indexmap::IndexMap;

pub type RouterWeights = IndexMap<String, IndexMap<String, f64>>;

#[derive(Debug, thiserror::Error)]
pub enum RouterWeightsError {
    #[error("router_settings must be an object")]
    SettingsNotAnObject,
    #[error("router_settings.weights must be an object or null")]
    WeightsNotAnObject,
    #[error("each router weight group must be an object")]
    GroupNotAnObject,
    #[error("router weight identifiers must contain a non-whitespace character")]
    InvalidIdentifier,
    #[error("router weights must be finite nonnegative numbers")]
    InvalidWeight,
    #[error("Each nonempty weights group must contain at least one positive weight")]
    NoPositiveWeight,
}

pub fn decode_router_weights(settings: &Json) -> Result<Option<RouterWeights>, RouterWeightsError> {
    let Json::Object(settings) = settings else {
        return Err(RouterWeightsError::SettingsNotAnObject);
    };
    let Some(weights) = settings.get("weights") else {
        return Ok(None);
    };
    match weights {
        Json::Null => Ok(None),
        Json::Object(groups) => decode_groups(groups).map(Some),
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Err(RouterWeightsError::WeightsNotAnObject)
        }
    }
}

fn decode_groups(groups: &JsonObject) -> Result<RouterWeights, RouterWeightsError> {
    let mut decoded = RouterWeights::new();
    for (model_group, group) in groups {
        validate_identifier(model_group)?;
        let Json::Object(entries) = group else {
            return Err(RouterWeightsError::GroupNotAnObject);
        };
        decoded.insert(model_group.clone(), decode_group(entries)?);
    }
    Ok(decoded)
}

fn decode_group(entries: &JsonObject) -> Result<IndexMap<String, f64>, RouterWeightsError> {
    let mut decoded = IndexMap::new();
    for (deployment_id, weight) in entries {
        validate_identifier(deployment_id)?;
        decoded.insert(deployment_id.clone(), decode_weight(weight)?);
    }
    if !decoded.is_empty() && !decoded.values().any(|weight| *weight > 0.0) {
        return Err(RouterWeightsError::NoPositiveWeight);
    }
    Ok(decoded)
}

fn validate_identifier(name: &str) -> Result<(), RouterWeightsError> {
    if name.chars().any(|character| !character.is_whitespace()) {
        return Ok(());
    }
    Err(RouterWeightsError::InvalidIdentifier)
}

fn decode_weight(value: &Json) -> Result<f64, RouterWeightsError> {
    let Json::Number(number) = value else {
        return Err(RouterWeightsError::InvalidWeight);
    };
    let Some(weight) = number.as_f64() else {
        return Err(RouterWeightsError::InvalidWeight);
    };
    if weight < 0.0 {
        return Err(RouterWeightsError::InvalidWeight);
    }
    Ok(weight)
}
