pub mod access;
pub mod cache;
pub mod error;
pub mod record;
pub mod selection;
pub mod store;

mod tests;

use uuid::Uuid;

pub const CACHE_KEY_PREFIX: &str = "litellm:polling:response:";
pub const POLLING_ID_PREFIX: &str = "litellm_poll_";
pub const DEFAULT_TTL: i64 = betterlite_config::api::responses_poll_state::DEFAULT_POLLING_TTL;

#[must_use]
// ast-grep-ignore: no-one-line-functions - the source keeps these as its public id helpers, and B938 dispatches on is_polling_id
pub fn generate_polling_id(uuid: Uuid) -> String {
    format!("{POLLING_ID_PREFIX}{uuid}")
}

#[must_use]
// ast-grep-ignore: no-one-line-functions - the source keeps these as its public id helpers, and B938 dispatches on is_polling_id
pub fn is_polling_id(response_id: &str) -> bool {
    response_id.starts_with(POLLING_ID_PREFIX)
}

#[must_use]
// ast-grep-ignore: no-one-line-functions - the source keeps these as its public id helpers, and B938 dispatches on is_polling_id
pub fn get_cache_key(polling_id: &str) -> String {
    format!("{CACHE_KEY_PREFIX}{polling_id}")
}
