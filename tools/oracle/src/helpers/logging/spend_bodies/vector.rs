use betterlite_logging::spend_bodies::placeholder::placeholder_request_body;
use betterlite_logging::spend_bodies::support::NodeAllocator;
use betterlite_logging::spend_bodies::vector_store::redact_vector_store;
use betterlite_logging::spend_sanitize::value::{SpendList, SpendValue, to_json};
use serde_json::{Value, json};

use super::support::{decode, invalid, mapping_json, mapping_of};

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VectorSpec {
    store_bodies: bool,
    vector_store: Option<Value>,
    litellm_params: Value,
    model_group: String,
    raw_model: String,
}

pub(super) fn vector_and_placeholder(spec: &VectorSpec) -> crate::helpers::HelperResult {
    let mut nodes = NodeAllocator::default();
    let vector_store = match &spec.vector_store {
        Some(value) => {
            let decoded = decode(value, &mut nodes)?;
            let SpendValue::List(list) = decoded else {
                return Err(invalid("vector_store must be a JSON list"));
            };
            let mut items = list.items;
            redact_vector_store(&mut items, spec.store_bodies);
            let redacted = SpendValue::List(SpendList {
                node: nodes.allocate(),
                items,
            });
            serde_json::to_value(to_json(&redacted)).map_err(|error| invalid(&error.to_string()))?
        }
        None => Value::Null,
    };
    let params = decode(&spec.litellm_params, &mut nodes)?;
    let placeholder = placeholder_request_body(
        mapping_of(&params)?,
        &spec.model_group,
        &spec.raw_model,
        &mut nodes,
    );
    Ok(json!({
        "vector_store": vector_store,
        "placeholder": mapping_json(&placeholder)?,
    }))
}
