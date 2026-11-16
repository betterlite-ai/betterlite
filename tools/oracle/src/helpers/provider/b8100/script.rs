use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::Object;
use secrecy::SecretString;
use serde::Deserialize;

use betterlite_oracle_helper_api::error::HelperError;

#[derive(Deserialize)]
pub(super) struct AgentCase {
    pub(super) name: String,
    pub(super) card: Object,
    pub(super) params: Object,
}

#[derive(Deserialize)]
pub(super) struct ProbeCase {
    pub(super) agent_name: String,
    pub(super) api_base: Option<String>,
    pub(super) api_key: Option<SecretString>,
    pub(super) headers: Option<BTreeMap<String, String>>,
    pub(super) optional_params: Object,
}

#[derive(Deserialize)]
pub(super) struct RequestCase {
    pub(super) model: String,
    pub(super) api_base: Option<String>,
    pub(super) api_key: Option<SecretString>,
    pub(super) headers: BTreeMap<String, String>,
    pub(super) non_default_params: Object,
    pub(super) optional_params: Object,
}

#[derive(Deserialize)]
pub(super) struct ResponseCase {
    pub(super) model: String,
    pub(super) status: u16,
    pub(super) body: String,
}

#[derive(Deserialize)]
pub(super) struct StreamCase {
    pub(super) chunk: Object,
}

pub(super) fn helper_error(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

pub(super) fn runtime() -> Result<tokio::runtime::Runtime, Box<HelperError>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|source| helper_error("Runtime", source.to_string()))
}
