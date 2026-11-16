use std::collections::BTreeMap;

use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

use crate::straiker::delivery::settings::{ApiVersion, StraikerSettings};

pub const CONTENT_TYPE_HEADER: &str = "Content-Type";
pub const CONTENT_TYPE_VALUE: &str = "application/json";
pub const WEBHOOK_FORMAT_HEADER: &str = "X-Straiker-Webhook-Format";
pub const WEBHOOK_FORMAT_VALUE: &str = "litellm";
const RESERVED_HEADERS: [&str; 3] = ["authorization", "content-type", "x-straiker-webhook-format"];

pub struct WebhookHeaders {
    pub authorization: SecretString,
    pub values: BTreeMap<String, String>,
}

#[must_use]
pub fn compose(
    settings: &StraikerSettings,
    extra: &IndexMap<&'static str, String>,
) -> WebhookHeaders {
    let mut values = BTreeMap::from([(
        CONTENT_TYPE_HEADER.to_owned(),
        CONTENT_TYPE_VALUE.to_owned(),
    )]);
    if settings.api_version != ApiVersion::V3 {
        values.insert(
            WEBHOOK_FORMAT_HEADER.to_owned(),
            WEBHOOK_FORMAT_VALUE.to_owned(),
        );
    }
    for (name, value) in &settings.custom_headers {
        if !RESERVED_HEADERS.contains(&name.to_lowercase().as_str()) {
            values.insert(name.clone(), value.clone());
        }
    }
    for (name, value) in extra {
        values.insert((*name).to_owned(), value.clone());
    }
    let key = settings.api_key.expose_secret();
    WebhookHeaders {
        authorization: SecretString::from(format!("Bearer {key}")),
        values,
    }
}
