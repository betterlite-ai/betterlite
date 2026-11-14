pub mod config;
pub mod create;
pub mod decode;
pub mod error;
pub mod request;

mod tests;

use std::collections::BTreeMap;

use secrecy::SecretString;

pub type ResponseHeaders = BTreeMap<String, String>;

// The litellm_params entries the File Search transformations read; everything else travels untouched.
#[derive(Debug, Clone, Default)]
pub struct RequestParams {
    pub api_key: Option<SecretString>,
    pub model: Option<String>,
}
