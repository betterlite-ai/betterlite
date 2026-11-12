use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;
use secrecy::SecretString;
use serde::Deserialize;

use crate::helpers::HelperError;

#[derive(Deserialize)]
pub(crate) struct InfoCase {
    pub(super) api_base: Option<String>,
    pub(super) api_key: Option<SecretString>,
    pub(super) env_api_base: Option<String>,
    pub(super) env_api_key: Option<SecretString>,
}

#[derive(Deserialize)]
pub(crate) struct UrlCase {
    pub(super) api_base: Option<String>,
    pub(super) stream: bool,
}

#[derive(Deserialize)]
pub(crate) struct HeadersCase {
    pub(super) api_key: Option<SecretString>,
    pub(super) headers: BTreeMap<String, String>,
}

#[derive(Deserialize)]
pub(crate) struct RequestCase {
    pub(super) model: String,
    pub(super) messages: Vec<Message>,
    pub(super) options: Object,
    pub(super) stream: bool,
}

#[derive(Deserialize)]
pub(crate) struct ReplyCase {
    pub(super) model: String,
    pub(super) body: String,
    pub(super) status: u16,
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
