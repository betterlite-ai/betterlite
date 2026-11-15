use std::fmt::{self, Formatter};

use secrecy::{ExposeSecret as _, SecretString};

use crate::openai_base_config::Headers;
use crate::videos::VideoOptions;

pub const SUPPORTED_PARAMS: [&str; 8] = [
    "model",
    "prompt",
    "input_reference",
    "seconds",
    "size",
    "characters",
    "user",
    "extra_headers",
];

pub const DEFAULT_API_BASE: &str = "https://api.openai.com/v1";
pub const MULTIPART_FORM_DATA: bool = true;

// one configured identity: each source is a separate slot so two identities can never share one
#[derive(Clone, Default)]
pub struct VideoCredentials {
    pub api_key: Option<SecretString>,
    pub params_api_key: Option<SecretString>,
    pub global_api_key: Option<SecretString>,
    pub openai_key: Option<SecretString>,
    pub environment_api_key: Option<SecretString>,
}

const fn redacted(value: &Option<SecretString>) -> Option<&'static str> {
    if value.is_some() {
        Some("[REDACTED]")
    } else {
        None
    }
}

impl fmt::Debug for VideoCredentials {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VideoCredentials")
            .field("api_key", &redacted(&self.api_key))
            .field("params_api_key", &redacted(&self.params_api_key))
            .field("global_api_key", &redacted(&self.global_api_key))
            .field("openai_key", &redacted(&self.openai_key))
            .field("environment_api_key", &redacted(&self.environment_api_key))
            .finish()
    }
}

fn first_secret(credentials: &VideoCredentials) -> Option<&SecretString> {
    let sources = [
        &credentials.api_key,
        &credentials.params_api_key,
        &credentials.global_api_key,
        &credentials.openai_key,
        &credentials.environment_api_key,
    ];
    sources
        .into_iter()
        .flatten()
        .find(|secret| !secret.expose_secret().is_empty())
}

// validate_environment: the first truthy key wins, and an absent key still writes Bearer None
#[must_use]
pub fn validate_environment(headers: Headers, credentials: &VideoCredentials) -> Headers {
    let authorization = match first_secret(credentials) {
        Some(secret) => format!("Bearer {}", secret.expose_secret()),
        None => "Bearer None".to_owned(),
    };
    let mut headers = headers;
    headers.insert("Authorization".to_owned(), authorization);
    headers
}

// get_complete_url defaults only a None api_base and strips every trailing slash
#[must_use]
pub fn complete_url(api_base: Option<&str>) -> String {
    let base = api_base.unwrap_or(DEFAULT_API_BASE);
    format!("{}/videos", base.trim_end_matches('/'))
}

// map_openai_params copies every option, supported or not, and drop_params changes nothing
pub fn map_params(options: &VideoOptions, mapped: &mut VideoOptions) {
    for (name, value) in options {
        mapped.insert(name.clone(), value.clone());
    }
}
