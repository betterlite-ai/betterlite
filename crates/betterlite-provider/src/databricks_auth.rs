pub mod api_base;
pub mod endpoint;
pub mod environment;
pub mod error;
pub mod oauth;
pub mod redaction;
pub mod url_parts;
pub mod user_agent;
pub mod validate;

use std::collections::BTreeMap;

pub type Headers = BTreeMap<String, String>;

mod tests;
