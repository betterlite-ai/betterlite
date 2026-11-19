use betterlite_models::decisions::Json;
use indexmap::IndexMap;

const SIGN_MASK: u64 = 0x7fff_ffff_ffff_ffff;

// Python truthiness where LiteLLM branches on it: empty containers, zero and "" are falsey
#[must_use]
pub(super) fn is_truthy(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Bool(flag) => *flag,
        // comparing the bits keeps NaN truthy and both zeroes falsey without a float comparison clippy denies
        Json::Number(number) => number
            .as_f64()
            .map_or(true, |float| (float.to_bits() & SIGN_MASK) != 0),
        Json::Text(text) => !text.is_empty(),
        Json::List(items) => !items.is_empty(),
        Json::Object(fields) => !fields.is_empty(),
    }
}

// model_group or deployment_model_name, and only a string survives either lookup
#[must_use]
pub(super) fn metadata_model_group(metadata: &IndexMap<String, Json>) -> Option<String> {
    let raw = metadata
        .get("model_group")
        .filter(|value| is_truthy(value))
        .or_else(|| metadata.get("deployment_model_name"));
    let Some(Json::Text(text)) = raw else {
        return None;
    };
    Some(text.clone())
}
