use secrecy::SecretString;
use serde_json::{Map, Value};

use crate::openai_base_config::Headers;
use crate::videos::config::VideoCredentials;

pub(super) fn options(pairs: &[(&str, Value)]) -> Map<String, Value> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

pub(super) fn body(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).unwrap()
}

pub(super) fn headers(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn credentials(
    api_key: Option<&str>,
    params_api_key: Option<&str>,
    global_api_key: Option<&str>,
    openai_key: Option<&str>,
    environment_api_key: Option<&str>,
) -> VideoCredentials {
    VideoCredentials {
        api_key: api_key.map(SecretString::from),
        params_api_key: params_api_key.map(SecretString::from),
        global_api_key: global_api_key.map(SecretString::from),
        openai_key: openai_key.map(SecretString::from),
        environment_api_key: environment_api_key.map(SecretString::from),
    }
}
