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
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Request {
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
    },
    Registry {
        providers: Vec<String>,
    },
}
pub(crate) struct Op;
impl Helper for Op {
    type Input = Input;
    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Request {
                query,
                optional_params,
                api_key,
                api_base,
                environment,
                headers,
                reply,
            } => {
                let config = TinyfishSearch::load(environment);
                let key = api_key.as_deref().map(SecretString::from);
                let validated = config
                    .validated_headers(headers.clone(), key.as_ref(), api_base.as_deref())
                    .or_value_error()?;
                let body = config
                    .transform_search_request(&RequestInput {
                        query,
                        optional_params,
                        api_key: key.as_ref(),
                        api_base: api_base.as_deref(),
                        headers: &validated,
                    })
                    .or_value_error()?;
                let url = config
                    .complete_url(
                        api_base.as_deref(),
                        optional_params,
                        Some(&body),
                        key.as_ref(),
                    )
                    .or_value_error()?;
                let params = serde_json::to_value(&body).or_value_error()?;
                let response = reply_answer(&config, reply.as_ref())?;
                Ok(json!({
                    "headers": validated,
                    "params": params,
                    "url": url,
                    "response": response,
                }))
            }
            Input::Registry { providers } => Ok(registry_answer(providers)),
        }
    }
}
fn reply_answer(config: &TinyfishSearch<'_>, reply: Option<&ReplyCall>) -> HelperResult {
    let Some(reply) = reply else {
        return Ok(Value::Null);
    };
    let headers = Headers::new();
    let response = config.transform_search_response(&ProviderResponse {
        status_code: reply.status,
        headers: &headers,
        text: &reply.body,
    });
    match response {
        Ok(response) => serde_json::to_value(response).or_value_error(),
        Err(TinyfishSearchError::Provider(error)) => Ok(json!({
            "error": {"status": error.status_code, "message": error.message},
        })),
        Err(error) => Err(error).or_value_error(),
    }
}
fn registry_answer(providers: &[String]) -> Value {
    let environment: BTreeMap<String, String> = BTreeMap::new();
    let registry = SearchRegistry {
        environment: &environment,
    };
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
    json!({
        "display": config.name.display_name(),
        "method": method,
        "rich": config.supports_rich_search_input(),
    })
}
mod tests;
