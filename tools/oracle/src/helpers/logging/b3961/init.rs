use betterlite_logging::http_analytics_loggers::athina::{ADDITIONAL_KEYS, AthinaSettings};
use betterlite_logging::http_analytics_loggers::greenscale::GreenscaleSettings;
use betterlite_logging::http_analytics_loggers::promptlayer::PromptLayerSettings;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::json;

use super::InitScenario;

pub(crate) fn athina_init(scenario: &InitScenario) -> serde_json::Value {
    let settings = AthinaSettings::from_environment(&scenario.environment);
    json!({
        "headers": api_key_headers("athina-api-key", settings.api_key.as_ref()),
        "url": settings.endpoint,
        "additional_keys": ADDITIONAL_KEYS,
    })
}

pub(crate) fn greenscale_init(scenario: &InitScenario) -> serde_json::Value {
    let settings = GreenscaleSettings::from_environment(&scenario.environment);
    json!({
        "headers": api_key_headers("api-key", settings.api_key.as_ref()),
        "url": settings.endpoint,
    })
}

pub(crate) fn promptlayer_init(scenario: &InitScenario) -> serde_json::Value {
    json!({
        "key": secret_json(
            PromptLayerSettings::from_environment(&scenario.environment)
                .api_key
                .as_ref(),
        ),
    })
}

fn secret_json(secret: Option<&SecretString>) -> Json {
    secret.map_or(Json::Null, |value| {
        Json::Text(value.expose_secret().to_owned())
    })
}

fn api_key_headers(name: &str, api_key: Option<&SecretString>) -> IndexMap<String, Json> {
    let mut headers = IndexMap::new();
    headers.insert(name.to_owned(), secret_json(api_key));
    headers.insert(
        "content-type".to_owned(),
        Json::Text("application/json".to_owned()),
    );
    headers
}
