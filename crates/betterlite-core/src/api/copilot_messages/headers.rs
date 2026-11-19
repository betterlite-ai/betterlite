use betterlite_anthropic::credentials::ANTHROPIC_VERSION;
use betterlite_anthropic::headers::request::ANTHROPIC_VERSION_HEADER;
use betterlite_openai::copilot::headers::copilot_default_headers;
use secrecy::SecretString;

use super::{API_VERSION, Headers};

pub(super) fn with_copilot_defaults(mut headers: Headers, key: Option<&SecretString>) -> Headers {
    // a cached record without a token renders as Bearer None, like the f-string in LiteLLM
    let token = key
        .cloned()
        .unwrap_or_else(|| SecretString::from(String::from("None")));
    for (name, value) in copilot_default_headers(&token, None) {
        if !headers.contains_key(&name) {
            headers.insert(name, value);
        }
    }
    headers.insert(
        String::from("openai-intent"),
        String::from("messages-proxy"),
    );
    headers.insert(
        String::from("x-interaction-type"),
        String::from("messages-proxy"),
    );
    headers.insert(
        String::from("x-github-api-version"),
        String::from(API_VERSION),
    );
    if !headers.contains_key(ANTHROPIC_VERSION_HEADER) {
        headers.insert(
            String::from(ANTHROPIC_VERSION_HEADER),
            String::from(ANTHROPIC_VERSION),
        );
    }
    headers
}
