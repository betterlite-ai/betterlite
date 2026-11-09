use std::collections::BTreeMap;

use betterlite_core::api::search_providers::fastcrw::FastcrwSearch;
use betterlite_core::api::search_providers::fastcrw::reply::decode_fastcrw_reply;
use betterlite_core::api::search_providers::firecrawl::FirecrawlSearch;
use betterlite_core::api::search_providers::firecrawl::reply::decode_firecrawl_reply;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::provider::{SearchProvider as _, SearchQuery};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ProviderName {
    Fastcrw,
    Firecrawl,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    TransformSearchRequest,
    TransformSearchResponse,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    provider: ProviderName,
    function: Function,
    #[serde(default)]
    query: Option<SearchQuery>,
    #[serde(default)]
    optional_params: Object,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    replies: Vec<Value>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input.function {
            Function::TransformSearchRequest => request(input),
            Function::TransformSearchResponse => responses(input),
        }
    }
}

fn request(input: &Input) -> HelperResult {
    let query = input
        .query
        .clone()
        .ok_or_else(|| failure("query is required"))?;
    let key = input
        .api_key
        .as_deref()
        .map(|text| SecretString::from(text.to_owned()));
    let base = input.api_base.as_deref();
    match input.provider {
        ProviderName::Fastcrw => {
            let search = FastcrwSearch::load(&input.environment);
            let headers = search
                .prepare_headers(Headers::new(), key.as_ref(), base)
                .map_err(failure)?;
            let body = search
                .transform_search_request(&query, &input.optional_params)
                .map_err(failure)?;
            let url = search
                .complete_url(base, &input.optional_params, Some(&body))
                .map_err(failure)?;
            Ok(json!({"url": url, "headers": headers, "body": body}))
        }
        ProviderName::Firecrawl => {
            let search = FirecrawlSearch::load(&input.environment);
            let headers = search
                .prepare_headers(Headers::new(), key.as_ref(), base)
                .map_err(failure)?;
            let body = search
                .transform_search_request(&query, &input.optional_params)
                .map_err(failure)?;
            let url = search
                .complete_url(base, &input.optional_params, Some(&body))
                .map_err(failure)?;
            Ok(json!({"url": url, "headers": headers, "body": body}))
        }
    }
}

fn responses(input: &Input) -> HelperResult {
    let mut parsed = Vec::new();
    for reply in &input.replies {
        let bytes = serde_json::to_vec(reply).map_err(failure)?;
        let response = match input.provider {
            ProviderName::Fastcrw => {
                serde_json::to_value(decode_fastcrw_reply(&bytes).map_err(failure)?)
                    .map_err(failure)?
            }
            ProviderName::Firecrawl => {
                serde_json::to_value(decode_firecrawl_reply(&bytes).map_err(failure)?)
                    .map_err(failure)?
            }
        };
        parsed.push(response);
    }
    Ok(json!({"responses": parsed}))
}

fn failure<Error>(error: Error) -> Box<HelperError>
where
    Error: std::fmt::Display,
{
    Box::new(HelperError {
        class: "SearchProviderError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
