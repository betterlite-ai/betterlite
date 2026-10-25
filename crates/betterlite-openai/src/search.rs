pub mod credentials;
pub mod provider;
pub mod records;

#[path = "search/credentials/tests.rs"]
mod search_credential;
#[path = "search/provider/tests.rs"]
mod search_hooks;
#[path = "search/records/tests.rs"]
mod search_types;
