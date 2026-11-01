use std::collections::BTreeMap;

use serde::Deserialize;

use super::dynamic::{MapValue, MappingOrScalar};
use crate::tiered_pricing::Scalar;

// Every field mirrors one key read from a request's logged metadata mapping; an unknown key is ignored.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Metadata {
    pub litellm_gateway_injected_cache: Option<Scalar>,
    pub compression_savings: Option<MappingOrScalar<CompressionSavings>>,
    pub guardrail_information: Option<GuardrailInformation>,
    pub autorouter_savings_estimate: Option<MappingOrScalar<AutorouterEstimate>>,
    pub autorouter_savings: Option<Scalar>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(try_from = "BTreeMap<String, Scalar>")]
pub struct CompressionSavings {
    pub tokens_saved: Option<Scalar>,
}

impl From<BTreeMap<String, Scalar>> for CompressionSavings {
    fn from(mut map: BTreeMap<String, Scalar>) -> Self {
        Self {
            tokens_saved: map.remove("tokens_saved"),
        }
    }
}

// A bare mapping is one entry, a list is many entries, and any other shape has none.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum GuardrailInformation {
    One(GuardrailEntry),
    Many(Vec<MappingOrScalar<GuardrailEntry>>),
    Other(Scalar),
}

#[derive(Clone, Debug, Deserialize)]
#[serde(try_from = "BTreeMap<String, MapValue>")]
pub struct GuardrailEntry {
    pub guardrail_provider: Option<Scalar>,
    pub guardrail_response: Option<MappingOrScalar<CompressionSavings>>,
}

impl From<BTreeMap<String, MapValue>> for GuardrailEntry {
    fn from(mut map: BTreeMap<String, MapValue>) -> Self {
        let guardrail_provider = match map.remove("guardrail_provider") {
            Some(MapValue::Scalar(scalar)) => Some(scalar),
            Some(MapValue::Mapping(_)) | None => None,
        };
        let guardrail_response = match map.remove("guardrail_response") {
            Some(MapValue::Mapping(inner)) => {
                Some(MappingOrScalar::Mapping(CompressionSavings::from(inner)))
            }
            Some(MapValue::Scalar(scalar)) => Some(MappingOrScalar::Other(scalar)),
            None => None,
        };
        Self {
            guardrail_provider,
            guardrail_response,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(try_from = "BTreeMap<String, Scalar>")]
pub struct AutorouterEstimate {
    pub version: Option<Scalar>,
    pub status: Option<Scalar>,
}

impl From<BTreeMap<String, Scalar>> for AutorouterEstimate {
    fn from(mut map: BTreeMap<String, Scalar>) -> Self {
        Self {
            version: map.remove("version"),
            status: map.remove("status"),
        }
    }
}

mod tests;
