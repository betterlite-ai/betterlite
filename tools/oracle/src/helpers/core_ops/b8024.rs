use std::collections::BTreeMap;

use betterlite_core::api::search_providers::google_pse::{GooglePseBody, GooglePseSearch};
use betterlite_core::api::search_providers::searchapi::{SearchApiBody, SearchApiSearch};
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
            ProviderName::GooglePse => request(
                input,
                &GooglePseSearch {
                    lookup: &input.environment,
                    api_key: input.api_key.clone().map(SecretString::from),
                    api_base: input.api_base.clone(),
                    search_engine_id: input.search_engine_id.clone(),
                },
            ),
            ProviderName::SearchApi => request(
                input,
                &SearchApiSearch {
                    lookup: &input.environment,
                    api_key: input.api_key.clone().map(SecretString::from),
                    api_base: input.api_base.clone(),
                },
            ),
        }
    }
}

type Outcome<T> = Result<T, Box<HelperError>>;

trait SearchRequest {
    type Body: serde::Serialize;

    fn validate(&self, headers: Headers) -> Outcome<Headers>;
    fn transform(&self, query: &SearchQuery, options: &Object) -> Outcome<Self::Body>;
    fn complete(&self, api_base: Option<&str>, body: &Self::Body) -> Outcome<String>;
}

impl SearchRequest for GooglePseSearch<'_> {
    type Body = GooglePseBody;

    fn validate(&self, headers: Headers) -> Outcome<Headers> {
        self.validate_environment(headers).map_err(value_error)
    }

    fn transform(&self, query: &SearchQuery, options: &Object) -> Outcome<Self::Body> {
        self.transform_search_request(query, options)
            .map_err(value_error)
    }

    fn complete(&self, api_base: Option<&str>, body: &Self::Body) -> Outcome<String> {
        self.complete_url(api_base, Some(body)).map_err(value_error)
    }
}

impl SearchRequest for SearchApiSearch<'_> {
    type Body = SearchApiBody;

    fn validate(&self, headers: Headers) -> Outcome<Headers> {
        self.validate_environment(headers).map_err(value_error)
    }

    fn transform(&self, query: &SearchQuery, options: &Object) -> Outcome<Self::Body> {
        self.transform_search_request(query, options)
            .map_err(value_error)
    }

    fn complete(&self, api_base: Option<&str>, body: &Self::Body) -> Outcome<String> {
        self.complete_url(api_base, Some(body)).map_err(value_error)
    }
}

fn request<P>(input: &Input, provider: &P) -> HelperResult
where
    P: SearchRequest,
{
    let headers = provider.validate(input.headers.clone())?;
    let body = provider.transform(&input.query, &input.optional_params)?;
    let url = provider.complete(input.api_base.as_deref(), &body)?;
    let params = serde_json::to_value(body).map_err(value_error)?;
    Ok(serde_json::json!({ "headers": headers, "params": params, "url": url }))
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
