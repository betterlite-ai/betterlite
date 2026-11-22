pub mod catalog;
pub mod response;
pub mod view;

mod tests;

// the read routes this task registers; both declare LiteLLM's user_api_key_auth dependency
pub const SETTINGS_PATH: &str = "/router/settings";
pub const FIELDS_PATH: &str = "/router/fields";
pub const ROUTE_PATHS: [&str; 2] = [SETTINGS_PATH, FIELDS_PATH];
