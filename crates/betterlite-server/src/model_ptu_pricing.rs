use betterlite_models::model_registration::JsonObject;

pub mod catalog;
pub mod delta;
pub mod error;
pub mod prices;
mod tests;
pub mod zeroed;

// LiteLLM's _PTU_PRICED_PAIR: the two fields that make a deployment bill reserved capacity
pub const PTU_PRICED_PAIR: [&str; 2] = ["ptu_count", "cost_per_ptu_per_hour"];

// both blobs of a deployment as LiteLLM dumps them with exclude_none, so a null reads as absent
#[derive(Clone, Copy, Debug)]
pub struct DeploymentBlobs<'blob> {
    pub model_info: &'blob JsonObject,
    pub litellm_params: &'blob JsonObject,
}
