mod common;
pub mod extraction;
pub mod helpers;
pub mod llm_call_event;
pub mod request_context;
pub mod request_identity;
pub mod session_id;
pub mod time_to_chunk;

pub const REQUESTER_METADATA_PATH: &str = "requester_metadata.";
pub const DEFAULT_BAGGAGE_METADATA_KEYS: &[&str] = &[
    "user_api_key_org_id",
    "user_api_key_user_id",
    "user_api_key_alias",
    "user_api_key_end_user_id",
    "requester_ip_address",
];
