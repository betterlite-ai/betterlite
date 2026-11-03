pub mod callback;
mod common;
pub mod identity;
pub mod request_data;

pub const REQUESTER_METADATA_PATH: &str = "requester_metadata.";
pub const DEFAULT_BAGGAGE_METADATA_KEYS: &[&str] = &[
    "user_api_key_org_id",
    "user_api_key_user_id",
    "user_api_key_alias",
    "user_api_key_end_user_id",
    "requester_ip_address",
];

#[path = "metadata/tests.rs"]
mod tests;
