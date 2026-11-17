use betterlite_models::identity::provider_id::ProviderId;

pub mod delete;
pub mod error;
pub mod headers;
pub mod instructions;
pub mod request;
pub mod settings;
pub mod url;

mod tests;

pub const PROVIDER: ProviderId = ProviderId::FireworksAi;
pub const SUPPORTS_NATIVE_WEBSOCKET: bool = false;
pub const SUPPORTS_NATIVE_FILE_SEARCH: bool = false;
