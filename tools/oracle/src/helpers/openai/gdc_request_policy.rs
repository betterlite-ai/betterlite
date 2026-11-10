use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::Section;
use betterlite_config::sections::gdc_settings::{CallSettings, Raw, Settings};
use betterlite_openai::chat::ChatMessage;
use betterlite_openai::chat_content::RefuseDownloads;
#[rustfmt::skip]
use betterlite_openai::gdc_policy::{audience, caching_enabled, complete_url, get_supported_openai_params, read_env_bool, transform_request};
#[rustfmt::skip]
use betterlite_openai::openai_base_config::{CatalogCapabilities, ConfigFamily, Headers, OpenAiConfig, OpenAiSettings};
use betterlite_prompts::arguments::json::Object;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

#[rustfmt::skip]
#[derive(Debug, Deserialize)] #[serde(deny_unknown_fields)]
pub(crate) struct Input { scenarios: Vec<Scenario> }

#[rustfmt::skip]
#[derive(Debug, Deserialize)] #[serde(deny_unknown_fields)]
struct Scenario {
    config: Value,
    #[serde(default)] environment: BTreeMap<String, String>,
    #[serde(default)] urls: Vec<UrlCall>,
    #[serde(default)] audiences: Vec<AudienceCall>,
    #[serde(default)] supported: Vec<SupportedCall>,
    #[serde(default)] settings: Vec<SettingsCall>,
    #[serde(default)] requests: Vec<RequestCall>,
}

#[rustfmt::skip]
#[derive(Debug, Deserialize)] #[serde(deny_unknown_fields)]
struct UrlCall { base: Option<String>, model: String, deployment: usize, #[serde(default)] optional_params: Object }

#[rustfmt::skip]
#[derive(Debug, Deserialize)] #[serde(deny_unknown_fields)]
struct AudienceCall { base: String, deployment: usize }

#[rustfmt::skip]
#[derive(Debug, Deserialize)] #[serde(deny_unknown_fields)]
struct SupportedCall { model: String, #[serde(default)] chat_models: BTreeSet<String>, #[serde(default)] text_models: BTreeSet<String> }

#[rustfmt::skip]
#[derive(Debug, Deserialize)] #[serde(deny_unknown_fields)]
struct SettingsCall { deployment: usize }

#[rustfmt::skip]
#[derive(Debug, Deserialize)] #[serde(deny_unknown_fields)]
struct RequestCall { model: String, #[serde(default)] messages: Vec<ChatMessage>, #[serde(default)] optional_params: Object }

impl Helper for Op {
    type Input = Input;

    #[rustfmt::skip]
    fn run(input: &Self::Input) -> HelperResult {
        let scenarios = input.scenarios.iter().map(scenario).collect::<Result<Vec<_>, _>>()?;
        Ok(json!({ "scenarios": scenarios }))
    }
}

#[rustfmt::skip]
fn deployment(settings: &Settings, index: usize) -> Result<&CallSettings, Box<HelperError>> {
    settings
        .deployments
        .get(index)
        .ok_or_else(|| Box::new(HelperError { class: "InvalidInput".to_owned(), status: None, error_type: None, param: None, code: None, message: format!("deployment {index} is not configured") }))
}

#[rustfmt::skip]
fn scenario(input: &Scenario) -> Result<Value, Box<HelperError>> {
    let raw = serde_json::from_value::<Raw>(input.config.clone())
        .map_err(|error| Box::new(HelperError { class: "InvalidInput".to_owned(), status: None, error_type: None, param: None, code: None, message: error.to_string() }))?;
    let loaded = Section::convert(raw, &input.environment)
        .map_err(|error| Box::new(HelperError { class: "InvalidInput".to_owned(), status: None, error_type: None, param: None, code: None, message: error.to_string() }))?;
    let urls = input.urls.iter().map(|call| url_outcome(call, &loaded)).collect::<Result<Vec<_>, _>>()?;
    let audiences = input.audiences.iter().map(|call| audience_outcome(call, &loaded)).collect::<Result<Vec<_>, _>>()?;
    let setting_values = input.settings.iter().map(|call| settings_outcome(call, &loaded)).collect::<Result<Vec<_>, _>>()?;
    let supported = input.supported.iter().map(supported_outcome).collect::<Vec<_>>();
    let requests = input.requests.iter().map(|call| request_outcome(call, &input.environment)).collect::<Vec<_>>();
    Ok(json!({"urls": urls, "audiences": audiences, "supported": supported, "settings": setting_values, "requests": requests}))
}

#[rustfmt::skip]
fn url_outcome(call: &UrlCall, settings: &Settings) -> Result<Value, Box<HelperError>> {
    let deployment = deployment(settings, call.deployment)?;
    match complete_url(call.base.as_deref(), deployment, &settings.global, &call.optional_params, &call.model) {
        Ok(url) => Ok(json!({"ok": true, "value": url})),
        Err(error) => Ok(json!({"ok": false, "status": error.class().metadata().status_code})),
    }
}

#[rustfmt::skip]
fn audience_outcome(call: &AudienceCall, settings: &Settings) -> Result<Value, Box<HelperError>> {
    deployment(settings, call.deployment)?;
    Ok(match audience(&call.base) {
        Ok(value) => json!({"ok": true, "value": value}),
        Err(error) => json!({"ok": false, "status": error.class().metadata().status_code}),
    })
}

#[rustfmt::skip]
fn settings_outcome(call: &SettingsCall, settings: &Settings) -> Result<Value, Box<HelperError>> {
    let deployment = deployment(settings, call.deployment)?;
    let ssl_verify = read_env_bool("SSL_VERIFY", deployment.ssl_verify.as_ref(), settings.ssl_verify_environment.as_deref(), true);
    let token_caching = read_env_bool("GDC_TOKEN_CACHING", deployment.gdc_token_caching.as_ref(), settings.gdc_token_caching_environment.as_deref(), false);
    Ok(json!({"ssl_verify": ssl_verify, "gdc_token_caching": token_caching, "caching_enabled": caching_enabled(&token_caching)}))
}

#[rustfmt::skip]
fn supported_outcome(call: &SupportedCall) -> Value {
    let config = OpenAiConfig { family: ConfigFamily::Other, api_key: None, headers: Headers::new(), settings: OpenAiSettings::default() };
    let catalog = CatalogCapabilities { chat_models: call.chat_models.clone(), text_models: call.text_models.clone() };
    json!(get_supported_openai_params(&call.model, &config, &catalog))
}

#[rustfmt::skip]
fn request_outcome(call: &RequestCall, environment: &BTreeMap<String, String>) -> Value {
    match transform_request(&call.model, &call.messages, &call.optional_params, environment, &RefuseDownloads) {
        Ok(body) => json!({"ok": true, "value": body}),
        Err(error) => json!({"ok": false, "status": error.status_code()}),
    }
}

mod tests;
