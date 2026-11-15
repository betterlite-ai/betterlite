use std::fmt::{self, Formatter};

use secrecy::{ExposeSecret as _, SecretString};

use serde_json::Value;

use crate::vector_stores::Headers;
use crate::videos::VideoOptions;
use crate::videos::create::VideoCreate;

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

const fn redacted(value: Option<&SecretString>) -> Option<&'static str> {
    if value.is_some() {
        Some("[REDACTED]")
    } else {
        None
    }
}

impl fmt::Debug for VideoCredentials {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("VideoCredentials")
            .field("api_key", &redacted(self.api_key.as_ref()))
            .field("params_api_key", &redacted(self.params_api_key.as_ref()))
            .field("global_api_key", &redacted(self.global_api_key.as_ref()))
            .field("openai_key", &redacted(self.openai_key.as_ref()))
            .field(
                "environment_api_key",
                &redacted(self.environment_api_key.as_ref()),
            )
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
pub fn validate_environment(mut headers: Headers, credentials: &VideoCredentials) -> Headers {
    let authorization = first_secret(credentials).map_or_else(
        || "Bearer None".to_owned(),
        |secret| format!("Bearer {}", secret.expose_secret()),
    );
    headers.insert(
        "Authorization".to_owned(),
        SecretString::from(authorization),
    );
    headers
}

// get_complete_url defaults only a None api_base and strips every trailing slash
#[must_use]
pub fn complete_url(api_base: Option<&str>) -> String {
    format!(
        "{}/videos",
        api_base.unwrap_or(DEFAULT_API_BASE).trim_end_matches('/')
    )
}

// map_openai_params copies every option, supported or not, and drop_params changes nothing
pub fn map_params(options: &VideoCreate, mapped: &mut VideoOptions) {
    for (name, value) in &options.extra {
        mapped.insert(name.clone(), value.clone());
    }
    for (name, value) in [
        ("seconds", &options.seconds),
        ("size", &options.size),
        ("user", &options.user),
    ] {
        if let Some(text) = value {
            mapped.insert(name.to_owned(), Value::String(text.clone()));
        }
    }
    if let Some(characters) = &options.characters {
        let rendered = characters.iter().map(|character| {
            let fields = character
                .iter()
                .map(|(key, text)| (key.clone(), Value::String(text.clone())));
            Value::Object(fields.collect())
        });
        mapped.insert("characters".to_owned(), Value::Array(rendered.collect()));
    }
}
