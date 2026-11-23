use betterlite_core::api::search_providers::registry::SearchRegistry;
use betterlite_core::api::search_providers::tinyfish::TinyfishSearch;
use betterlite_core::api::search_providers::tinyfish::error::TinyfishSearchError;
use betterlite_core::search::SearchConfigLookup as _;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::provider::{
    HttpMethod, ProviderResponse, RequestInput, SearchProvider as _, SearchQuery,
};
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplyCall {
    status: u16,
    body: String,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequestCall {
    query: SearchQuery,
    #[serde(default)]
    optional_params: Object,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    #[serde(default)]
    headers: Headers,
    #[serde(default)]
    reply: Option<ReplyCall>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RegistryCall {
    providers: Vec<String>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
}
#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(crate) enum Input {
    Request(RequestCall),
    Registry(RegistryCall),
}
pub(crate) struct Op;
impl Helper for Op {
    type Input = Input;
    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Request(call) => request_answer(call),
            Input::Registry(call) => Ok(registry_answer(&call.providers, &call.environment)),
        }
    }
}
fn request_answer(call: &RequestCall) -> HelperResult {
    let config = TinyfishSearch::load(&call.environment);
    let key = call.api_key.as_deref().map(SecretString::from);
    let validated = config
        .validated_headers(call.headers.clone(), key.as_ref(), call.api_base.as_deref())
        .or_value_error()?;
    let body = config
        .transform_search_request(&RequestInput {
            query: &call.query,
            optional_params: &call.optional_params,
            api_key: key.as_ref(),
            api_base: call.api_base.as_deref(),
            headers: &validated,
        })
        .or_value_error()?;
    let url = config
        .complete_url(
            call.api_base.as_deref(),
            &call.optional_params,
            Some(&body),
            key.as_ref(),
        )
        .or_value_error()?;
    let params = serde_json::to_value(&body).or_value_error()?;
    let response = reply_answer(&config, call.reply.as_ref())?;
    Ok(json!({
        "headers": validated,
        "params": params,
        "url": url,
        "response": response,
    }))
}
fn reply_answer(config: &TinyfishSearch<'_>, reply: Option<&ReplyCall>) -> HelperResult {
    let Some(present) = reply else {
        return Ok(Value::Null);
    };
    let headers = Headers::new();
    let response = config.transform_search_response(&ProviderResponse {
        status_code: present.status,
        headers: &headers,
        text: &present.body,
    });
    match response {
        Ok(value) => serde_json::to_value(value).or_value_error(),
        Err(TinyfishSearchError::Provider(error)) => Ok(json!({
            "error": {"status": error.status_code, "message": error.message},
        })),
        Err(error) => Err(error).or_value_error(),
    }
}
fn registry_answer(providers: &[String], environment: &BTreeMap<String, String>) -> Value {
    let registry = SearchRegistry { environment };
    let views: Vec<Value> = providers
        .iter()
        .map(|provider| registry_value(&registry, provider))
        .collect();
    json!({"providers": views})
}
fn registry_value(registry: &SearchRegistry<'_>, provider: &str) -> Value {
    let Some(config) = registry.search_config(provider) else {
        return Value::Null;
    };
    let method = match config.http_method() {
        HttpMethod::Get => "GET",
        HttpMethod::Post => "POST",
    };
    let endpoint = config.complete_url(None, &Object::new(), None, None).ok();
    json!({
        "display": config.name.display_name(),
        "method": method,
        "rich": config.supports_rich_search_input(),
        "endpoint": endpoint,
    })
}
mod tests;
