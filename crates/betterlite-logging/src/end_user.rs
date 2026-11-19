use betterlite_models::model_registration::JsonObject;

use crate::spend_metadata::{SpendMetadataError, merged_spend_metadata, optional_text_field};

pub const END_USER_ID_FIELD: &str = "user_api_key_end_user_id";

// litellm.disable_end_user_cost_tracking and litellm.enable_end_user_cost_tracking_prometheus_only, both Optional[bool] = None.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EndUserCostTrackingSettings {
    pub disable_end_user_cost_tracking: Option<bool>,
    pub enable_prometheus_only: Option<bool>,
}

// The independent disable_end_user_cost_tracking_prometheus_only setting stays unread here: it is not this function's inverse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CostTrackingService {
    LitellmLogging,
    Prometheus,
}

#[derive(Debug, thiserror::Error)]
pub enum EndUserIdError {
    #[error(transparent)]
    Metadata(#[from] SpendMetadataError),
}

// ast-grep-ignore: no-long-names
// get_end_user_id_for_cost_tracking: the top-level id wins when truthy, else the merged bucket's value, and the two
// settings gate the result.
pub fn get_end_user_id_for_cost_tracking(
    litellm_params: &JsonObject,
    service: CostTrackingService,
    settings: &EndUserCostTrackingSettings,
) -> Result<Option<String>, EndUserIdError> {
    let metadata = merged_spend_metadata(litellm_params)?;
    let top_level = optional_text_field(litellm_params.get(END_USER_ID_FIELD), END_USER_ID_FIELD)?;
    let merged = optional_text_field(metadata.get(END_USER_ID_FIELD), END_USER_ID_FIELD)?;
    let end_user_id = match top_level {
        Some(id) if !id.is_empty() => Some(id.to_owned()),
        Some(_) | None => merged.map(ToOwned::to_owned),
    };
    if settings.disable_end_user_cost_tracking == Some(true) {
        return Ok(None);
    }
    if service == CostTrackingService::Prometheus && settings.enable_prometheus_only != Some(true) {
        return Ok(None);
    }
    Ok(end_user_id)
}

#[path = "end_user/tests.rs"]
mod tests;
