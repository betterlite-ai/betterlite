pub mod auth;
pub mod decisions;
pub mod path;
pub mod url;

mod tests;

use betterlite_models::identity::provider_id::ProviderId;

pub const PROVIDER: ProviderId = ProviderId::AzureAi;
pub const SUPPORTS_NATIVE_WEBSOCKET: bool = false;
