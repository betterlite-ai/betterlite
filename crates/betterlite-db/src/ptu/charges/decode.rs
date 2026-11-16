use std::borrow::Cow;
use std::collections::BTreeMap;

use betterlite_models::tiered_pricing::Scalar;
use serde::de::IgnoredAny;

use crate::budget_permission_rows::JsonData;
use crate::resource_rows::parse_data;

pub type ModelInfo = BTreeMap<String, JsonData>;

// LiteLLM decodes a stored model_info string once, and only an object survives the decode.
pub fn decode_ptu_model_info(raw: &JsonData) -> Option<Cow<'_, ModelInfo>> {
    match raw {
        JsonData::Object(mapping) => Some(Cow::Borrowed(mapping)),
        JsonData::String(text) => decode_model_info_text(text).map(Cow::Owned),
        JsonData::Null | JsonData::Bool(_) | JsonData::Number(_) | JsonData::Array(_) => None,
    }
}

fn decode_model_info_text(text: &str) -> Option<ModelInfo> {
    match parse_data(text.as_bytes()).ok()? {
        JsonData::Object(mapping) => Some(mapping),
        JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::String(_)
        | JsonData::Array(_) => None,
    }
}

#[must_use]
pub fn model_info_scalars(model_info: &ModelInfo) -> BTreeMap<String, Scalar> {
    model_info
        .iter()
        .map(|(name, value)| (name.clone(), scalar_view(value)))
        .collect()
}

// PTU term validation reads a scalar's shape only, so an array or object keeps its kind and drops its contents.
fn scalar_view(value: &JsonData) -> Scalar {
    match value {
        JsonData::Null => Scalar::Null,
        JsonData::Bool(flag) => Scalar::Bool(*flag),
        JsonData::Number(number) => Scalar::Number(number.clone()),
        JsonData::String(text) => Scalar::Text(text.clone()),
        JsonData::Array(_) => Scalar::Array(Vec::new()),
        JsonData::Object(_) => Scalar::Object(IgnoredAny),
    }
}
