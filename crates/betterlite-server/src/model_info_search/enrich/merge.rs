use betterlite_models::decisions::Json;
use betterlite_models::model_registration::{JsonObject, ModelRegistry};
use betterlite_models::request_pricing::fields::{
    PRICING_OVERRIDES_KEY, is_server_derived_pricing_key, pricing_override_fields,
};

use crate::model_info_scope::text_field;

// _pricing_override_stamps: the deployment's own server-derived prices, then the sorted field names they override.
fn pricing_override_stamps(model_info: &JsonObject, params: &JsonObject) -> JsonObject {
    let mut own = JsonObject::new();
    for (key, value) in params {
        if !matches!(value, Json::Null) && is_server_derived_pricing_key(key) {
            own.insert(key.clone(), value.clone());
        }
    }
    let mut stamps = own.clone();
    let overrides = pricing_override_fields(&[model_info, &own]);
    stamps.insert(
        PRICING_OVERRIDES_KEY.to_owned(),
        Json::List(overrides.into_iter().map(Json::Text).collect()),
    );
    stamps
}

// The merge keeps every stamped field, fills a stamped null only from discovery, and leaves inserted token prices null when unpriced.
pub(super) fn merge_model_info(
    model_info: &JsonObject,
    params: &JsonObject,
    catalogue: &JsonObject,
    discovered: &JsonObject,
    registry: &ModelRegistry,
) -> JsonObject {
    let mut stamped = model_info.clone();
    for (key, value) in pricing_override_stamps(model_info, params) {
        stamped.insert(key, value);
    }
    let id = text_field(model_info, "id");
    let key = text_field(catalogue, "key");
    let keys: Vec<&str> = [id, key].into_iter().flatten().collect();
    let unpriced = registry.cost_map_omits_token_price(&keys);
    let mut combined = catalogue.clone();
    for (name, value) in discovered {
        combined.insert(name.clone(), value.clone());
    }
    for (name, value) in combined {
        let fill = match stamped.get(&name) {
            None => true,
            Some(Json::Null) => discovered.contains_key(&name),
            Some(
                Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_),
            ) => false,
        };
        if !fill {
            continue;
        }
        let value = if unpriced
            && matches!(
                name.as_str(),
                "input_cost_per_token" | "output_cost_per_token"
            ) {
            Json::Null
        } else {
            value
        };
        stamped.insert(name, value);
    }
    stamped
}
