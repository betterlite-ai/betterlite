use std::collections::BTreeMap;

use betterlite_core::api::search_providers::google_pse::GooglePseSearch;
use betterlite_core::api::search_providers::searchapi::SearchApiSearch;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::provider::SearchQuery;
use secrecy::SecretString;
use serde::Deserialize;

use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Debug, Clone, Copy, Deserialize)]
enum ProviderName {
    #[serde(rename = "google_pse")]
    GooglePse,
    #[serde(rename = "searchapi")]
    SearchApi,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    provider: ProviderName,
    query: SearchQuery,
    #[serde(default)]
    optional_params: Object,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    search_engine_id: Option<String>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    #[serde(default)]
    headers: Headers,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        match input.provider {
            ProviderName::GooglePse => google_pse(input),
            ProviderName::SearchApi => search_api(input),
        }
    }
}

fn google_pse(input: &Input) -> HelperResult {
    let provider = GooglePseSearch {
        lookup: &input.environment,
        api_key: input.api_key.clone().map(SecretString::from),
        api_base: input.api_base.clone(),
        search_engine_id: input.search_engine_id.clone(),
    };
    let headers = provider
        .validate_environment(input.headers.clone())
        .map_err(value_error)?;
    let body = provider
        .transform_search_request(&input.query, &input.optional_params)
        .map_err(value_error)?;
    let url = provider
        .complete_url(input.api_base.as_deref(), Some(&body))
        .map_err(value_error)?;
    Ok(serde_json::json!({ "headers": headers, "params": body, "url": url }))
}

fn search_api(input: &Input) -> HelperResult {
    let provider = SearchApiSearch {
        lookup: &input.environment,
        api_key: input.api_key.clone().map(SecretString::from),
        api_base: input.api_base.clone(),
    };
    let headers = provider
        .validate_environment(input.headers.clone())
        .map_err(value_error)?;
    let body = provider
        .transform_search_request(&input.query, &input.optional_params)
        .map_err(value_error)?;
    let url = provider
        .complete_url(input.api_base.as_deref(), Some(&body))
        .map_err(value_error)?;
    Ok(serde_json::json!({ "headers": headers, "params": body, "url": url }))
}

fn value_error<E>(error: E) -> Box<HelperError>
where
    E: std::fmt::Display,
{
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
