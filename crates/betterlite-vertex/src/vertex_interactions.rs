pub mod config;
pub mod error;
pub mod mint;
pub mod request;
pub mod target;

mod tests;

use betterlite_models::identity::provider_id::ProviderId;

pub const API_VERSION: &str = "v1beta1";
pub const DEFAULT_LOCATION: &str = "global";
// the Vertex interactions config reports the same provider LiteLLM's custom_llm_provider does
pub const PROVIDER: ProviderId = ProviderId::VertexAi;
