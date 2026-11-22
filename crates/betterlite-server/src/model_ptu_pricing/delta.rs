use std::collections::BTreeSet;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::ptu_pricing::pricing_fields::{
    CUSTOM_PRICING_FIELDS, TIERED_PRICING_FIELD, ZEROED_RATE_FIELDS,
};
use betterlite_models::ptu_pricing::zeroed::PtuZeroedPricing;

use super::error::PtuPricingError;
use super::prices::is_zero_price;
use super::zeroed::ptu_zeroed_pricing;
use super::{DeploymentBlobs, PTU_PRICED_PAIR};
use crate::jobs::ptu::attribution::PtuAttribution;
use crate::model_effective_fields::ptu::explicitly_cleared_ptu_fields;

// the two blobs of an updateDeployment body; an absent one patches nothing
#[derive(Clone, Copy, Debug)]
pub struct PtuPatch<'patch> {
    pub litellm_params: Option<&'patch JsonObject>,
    pub model_info: Option<&'patch JsonObject>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PtuPricingDelta {
    pub written: PtuZeroedPricing,
    pub released: BTreeSet<&'static str>,
}

// a patch that takes a deployment off PTU drops the zeros PTU wrote, or the deployment would serve for free
pub fn ptu_pricing_delta(
    stored_model_info: &JsonObject,
    blobs: DeploymentBlobs<'_>,
    patch: PtuPatch<'_>,
    attribution: &dyn PtuAttribution,
) -> Result<PtuPricingDelta, PtuPricingError> {
    let nothing_supplied = JsonObject::new();
    let supplied = patch.litellm_params.unwrap_or(&nothing_supplied);
    let written = ptu_zeroed_pricing(blobs, supplied, attribution)?;
    let mut released = BTreeSet::new();
    if !written.entries.is_empty() {
        return Ok(PtuPricingDelta { written, released });
    }
    let was_ptu = PTU_PRICED_PAIR.into_iter().any(|field| {
        stored_model_info
            .get(field)
            .is_some_and(|value| !matches!(value, Json::Null))
    });
    let leaves_ptu = explicitly_cleared_ptu_fields(patch.model_info, attribution)
        .iter()
        .any(|field| PTU_PRICED_PAIR.contains(field));
    if !was_ptu || !leaves_ptu {
        return Ok(PtuPricingDelta { written, released });
    }
    // a zero the patch carries goes too, since the dashboard echoes the stored blob and its zeros on every save
    let releasable = CUSTOM_PRICING_FIELDS
        .into_iter()
        .chain(ZEROED_RATE_FIELDS)
        .chain([TIERED_PRICING_FIELD]);
    for field in releasable {
        let zeroed = [blobs.model_info, blobs.litellm_params]
            .into_iter()
            .any(|blob| blob.get(field).is_some_and(is_zero_price));
        if zeroed {
            released.insert(field);
        }
    }
    Ok(PtuPricingDelta { written, released })
}
