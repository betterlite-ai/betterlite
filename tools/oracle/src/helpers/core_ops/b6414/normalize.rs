use betterlite_core::meta_muse_realtime::session::build_muse_realtime_url;
use betterlite_core::meta_muse_realtime::session::get_complete_url;
use betterlite_core::meta_muse_realtime::session::normalize_access_token;
use betterlite_core::meta_muse_realtime::session::normalize_language;
use secrecy::ExposeSecret as _;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;

#[derive(Debug, Deserialize)]
pub(crate) struct NormalizeInput {
    languages: Vec<String>,
    tokens: Vec<String>,
    urls: Vec<Option<String>>,
    models: Vec<String>,
}

pub(crate) fn run(fields: &NormalizeInput) -> Value {
    let languages: Vec<Option<String>> = fields
        .languages
        .iter()
        .map(|language| normalize_language(language).ok())
        .collect();
    let tokens: Vec<Option<String>> = fields
        .tokens
        .iter()
        .map(|token| token_document(token.as_str()))
        .collect();
    let urls: Vec<Option<String>> = fields
        .urls
        .iter()
        .map(|base| build_muse_realtime_url(base.as_deref()).ok())
        .collect();
    let models: Vec<Option<String>> = fields
        .models
        .iter()
        .map(|model| get_complete_url(None, model).ok())
        .collect();
    json!({
        "languages": languages,
        "tokens": tokens,
        "urls": urls,
        "models": models,
    })
}

fn token_document(token: &str) -> Option<String> {
    let secret = SecretString::from(token.to_owned());
    normalize_access_token(&secret)
        .ok()
        .map(|normalized| normalized.expose_secret().to_owned())
}
