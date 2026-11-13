use std::collections::{BTreeMap, BTreeSet};

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::ptu_pricing::zeroed::{PtuZero, zeroed_ptu_pricing};
use betterlite_models::ptu_pricing::{
    PtuModelInfo, ptu_config_error, ptu_identity_error, ptu_terms,
};
use betterlite_models::tiered_pricing::Scalar;

use super::construct::DeploymentInput;
use super::error::InvalidDeployment;

// The search context prices a PTU deployment zeroes.
const SEARCH_CONTEXT_SIZES: [&str; 3] = [
    "search_context_size_low",
    "search_context_size_medium",
    "search_context_size_high",
];

// A config-sourced deployment that declares PTU terms must pass the PTU rules and carry its own unique id.
pub(super) fn config_error(
    input: &DeploymentInput,
    info: &BTreeMap<String, Scalar>,
) -> Option<String> {
    let ptu = PtuModelInfo::from(info);
    let taken = input
        .declared_id
        .as_ref()
        .is_some_and(|id| input.duplicate_ids.contains(id));
    let current_id = match input.model_info.get("id") {
        Some(Json::Text(id)) => Some(id.as_str()),
        _ => None,
    };
    let identity = ptu_terms(&ptu).and_then(|_| {
        ptu_identity_error(
            input.declared_id.as_deref(),
            taken,
            current_id,
            Some(&input.model_name),
        )
    });
    ptu_config_error(&ptu, Some(&input.model_name)).or(identity)
}

// The rates a PTU deployment must not bill: zeroed pricing for the standing set and for any rate it declares itself.
pub(super) fn zeroed_pricing(
    input: &DeploymentInput,
    info: &BTreeMap<String, Scalar>,
    enabled: bool,
) -> Result<JsonObject, InvalidDeployment> {
    let declared: BTreeSet<String> = input.litellm_params.keys().cloned().collect();
    let mut zeroed = JsonObject::new();
    let Some(pricing) = zeroed_ptu_pricing(&PtuModelInfo::from(info), &declared, enabled) else {
        return Ok(zeroed);
    };
    let rate: Json = serde_json::from_str("0.0")?;
    for (field, zero) in pricing.entries {
        let value = match zero {
            PtuZero::Rate(_) => rate.clone(),
            PtuZero::EmptyTiers => Json::List(Vec::new()),
            PtuZero::SearchContext(_) => Json::Object(
                SEARCH_CONTEXT_SIZES
                    .iter()
                    .map(|size| ((*size).to_owned(), rate.clone()))
                    .collect(),
            ),
        };
        zeroed.insert(field.to_owned(), value);
    }
    Ok(zeroed)
}
