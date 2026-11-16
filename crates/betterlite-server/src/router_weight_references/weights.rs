use std::borrow::Borrow;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Number;

#[derive(Debug, thiserror::Error)]
pub enum RouterWeightValueError {
    #[error("router weight identifiers must contain a non-whitespace character")]
    Identifier,
    #[error("router weights must be finite nonnegative numbers")]
    Weight,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Hash)]
#[serde(try_from = "String")]
pub struct RouterWeightIdentifier(String);

impl TryFrom<String> for RouterWeightIdentifier {
    type Error = RouterWeightValueError;

    fn try_from(name: String) -> Result<Self, Self::Error> {
        if name.chars().all(char::is_whitespace) {
            return Err(RouterWeightValueError::Identifier);
        }
        Ok(Self(name))
    }
}

impl RouterWeightIdentifier {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Borrow<str> for RouterWeightIdentifier {
    fn borrow(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(try_from = "Number")]
pub struct RouterWeight(f64);

impl TryFrom<Number> for RouterWeight {
    type Error = RouterWeightValueError;

    fn try_from(number: Number) -> Result<Self, Self::Error> {
        match number.as_f64() {
            Some(weight) if weight >= 0.0 => Ok(Self(weight)),
            _ => Err(RouterWeightValueError::Weight),
        }
    }
}

impl RouterWeight {
    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }
}

pub type RouterWeights =
    IndexMap<RouterWeightIdentifier, IndexMap<RouterWeightIdentifier, RouterWeight>>;

#[derive(Debug, thiserror::Error)]
pub enum RouterWeightsError {
    #[error("router_settings must be an object")]
    SettingsNotAnObject,
    #[error("router_settings.weights is invalid: {0}")]
    Decode(serde_json::Error),
    #[error("Each nonempty weights group must contain at least one positive weight")]
    NoPositiveWeight,
}

#[derive(Deserialize)]
struct RouterWeightSettings {
    #[serde(default)]
    weights: Option<RouterWeights>,
}

pub fn decode_router_weights(settings: &Json) -> Result<Option<RouterWeights>, RouterWeightsError> {
    // serde would also read this one-field struct from a JSON array, so objects are checked first
    match settings {
        Json::Null => return Ok(None),
        Json::Object(_) => {}
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            return Err(RouterWeightsError::SettingsNotAnObject);
        }
    }
    let text = serde_json::to_string(settings).map_err(RouterWeightsError::Decode)?;
    let parsed: RouterWeightSettings =
        serde_json::from_str(&text).map_err(RouterWeightsError::Decode)?;
    let Some(weights) = parsed.weights else {
        return Ok(None);
    };
    let without_positive = weights
        .values()
        .any(|group| !group.is_empty() && group.values().all(|weight| weight.get() <= 0.0));
    if without_positive {
        return Err(RouterWeightsError::NoPositiveWeight);
    }
    Ok(Some(weights))
}
