use std::collections::BTreeMap;

use betterlite_core::api::search_providers::apiserpent::ApiserpentSearchConfig;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::provider::{ProviderResponse, SearchProvider as _, SearchQuery};
use secrecy::SecretString;
use serde::Deserialize;

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
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
    reply: Option<String>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        let config = ApiserpentSearchConfig::load(&input.environment);
        let key = input.api_key.clone().map(SecretString::from);
        let headers = config
            .validated_headers(
                input.headers.clone(),
                key.as_ref(),
                input.api_base.as_deref(),
            )
            .or_value_error()?;
        let body = config
            .transform_search_request(&input.query, &input.optional_params)
            .or_value_error()?;
        let url = config
            .complete_url(
                input.api_base.as_deref(),
                &input.optional_params,
                Some(&body),
            )
            .or_value_error()?;
        let params = serde_json::to_value(&body).or_value_error()?;
        let response = match &input.reply {
            Some(text) => {
                let empty_headers = Headers::new();
                let reply = config
                    .transform_search_response(&ProviderResponse {
                        status_code: 200,
                        headers: &empty_headers,
                        text,
                    })
                    .or_value_error()?;
                serde_json::to_value(&reply).or_value_error()?
            }
            None => serde_json::json!(null),
        };
        Ok(serde_json::json!({
            "headers": headers,
            "params": params,
            "url": url,
            "response": response,
        }))
    }
}

mod tests;
