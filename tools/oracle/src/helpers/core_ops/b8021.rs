use std::collections::BTreeMap;

use betterlite_core::api::search_providers::tavily::TavilySearch;
use betterlite_core::api::search_providers::you_com::YouComSearch;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::provider::{ProviderResponse, SearchProvider as _, SearchQuery};
use betterlite_openai::search::records::SearchResponse;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    provider: Provider,
    calls: Vec<Call>,
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Provider {
    Tavily,
    YouCom,
}

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
enum Call {
    ValidateEnvironment {
        #[serde(default)]
        headers: Headers,
        #[serde(default)]
        api_key: Option<String>,
        #[serde(default)]
        api_base: Option<String>,
        #[serde(default)]
        environment: BTreeMap<String, String>,
    },
    CompleteUrl {
        #[serde(default)]
        api_base: Option<String>,
        #[serde(default)]
        environment: BTreeMap<String, String>,
    },
    TransformSearchRequest {
        query: SearchQuery,
        #[serde(default)]
        optional_params: Object,
        #[serde(default)]
        environment: BTreeMap<String, String>,
    },
    TransformSearchResponse {
        #[serde(default = "default_status")]
        status: u16,
        #[serde(default)]
        headers: Headers,
        body: Value,
        #[serde(default)]
        environment: BTreeMap<String, String>,
    },
}

const fn default_status() -> u16 {
    200
}

enum ProviderSearch {
    Tavily(TavilySearch),
    YouCom(YouComSearch),
}

impl ProviderSearch {
    fn load(provider: Provider, environment: &BTreeMap<String, String>) -> Self {
        match provider {
            Provider::Tavily => Self::Tavily(TavilySearch::load(environment)),
            Provider::YouCom => Self::YouCom(YouComSearch::load(environment)),
        }
    }

    fn prepare_headers(
        &self,
        headers: Headers,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> Result<Headers, Box<HelperError>> {
        match self {
            Self::Tavily(search) => search
                .prepare_headers(headers, api_key, api_base)
                .map_err(failure),
            Self::YouCom(search) => search
                .prepare_headers(headers, api_key, api_base)
                .map_err(failure),
        }
    }

    fn complete_url(&self, api_base: Option<&str>) -> Result<String, Box<HelperError>> {
        match self {
            Self::Tavily(search) => search
                .complete_url(api_base, &Object::new(), None)
                .map_err(failure),
            Self::YouCom(search) => search
                .complete_url(api_base, &Object::new(), None)
                .map_err(failure),
        }
    }

    fn transform_search_request(
        &self,
        query: &SearchQuery,
        optional_params: &Object,
    ) -> Result<Object, Box<HelperError>> {
        match self {
            Self::Tavily(search) => search
                .transform_search_request(query, optional_params)
                .map_err(failure),
            Self::YouCom(search) => search
                .transform_search_request(query, optional_params)
                .map_err(failure),
        }
    }

    fn transform_search_response(
        &self,
        response: &ProviderResponse<'_>,
    ) -> Result<SearchResponse, Box<HelperError>> {
        match self {
            Self::Tavily(search) => search.transform_search_response(response).map_err(failure),
            Self::YouCom(search) => search.transform_search_response(response).map_err(failure),
        }
    }
}

#[derive(Serialize)]
struct Outcome {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<Value>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let outcomes: Vec<Outcome> = input
            .calls
            .iter()
            .map(|call| {
                call_outcome(input.provider, call).map_or_else(
                    |_error| Outcome {
                        ok: false,
                        value: None,
                    },
                    |value| Outcome {
                        ok: true,
                        value: Some(value),
                    },
                )
            })
            .collect();
        serde_json::to_value(outcomes).map_err(failure)
    }
}

fn call_outcome(provider: Provider, call: &Call) -> Result<Value, Box<HelperError>> {
    match call {
        Call::ValidateEnvironment {
            headers,
            api_key,
            api_base,
            environment,
        } => validate_called(
            provider,
            headers,
            api_key.as_deref(),
            api_base.as_deref(),
            environment,
        ),
        Call::CompleteUrl {
            api_base,
            environment,
        } => {
            let url =
                ProviderSearch::load(provider, environment).complete_url(api_base.as_deref())?;
            Ok(Value::String(url))
        }
        Call::TransformSearchRequest {
            query,
            optional_params,
            environment,
        } => request_called(provider, query, optional_params, environment),
        Call::TransformSearchResponse {
            status,
            headers,
            body,
            environment,
        } => response_called(provider, *status, headers, body, environment),
    }
}

fn validate_called(
    provider: Provider,
    headers: &Headers,
    api_key: Option<&str>,
    api_base: Option<&str>,
    environment: &BTreeMap<String, String>,
) -> Result<Value, Box<HelperError>> {
    let key = api_key.map(SecretString::from);
    let prepared = ProviderSearch::load(provider, environment).prepare_headers(
        headers.clone(),
        key.as_ref(),
        api_base,
    )?;
    serde_json::to_value(prepared).map_err(failure)
}

fn request_called(
    provider: Provider,
    query: &SearchQuery,
    optional_params: &Object,
    environment: &BTreeMap<String, String>,
) -> Result<Value, Box<HelperError>> {
    let body = ProviderSearch::load(provider, environment)
        .transform_search_request(query, optional_params)?;
    Ok(Value::Object(body))
}

fn response_called(
    provider: Provider,
    status: u16,
    headers: &Headers,
    body: &Value,
    environment: &BTreeMap<String, String>,
) -> Result<Value, Box<HelperError>> {
    let text = serde_json::to_string(body).map_err(failure)?;
    let response = ProviderResponse {
        status_code: status,
        headers,
        text: &text,
    };
    let parsed =
        ProviderSearch::load(provider, environment).transform_search_response(&response)?;
    serde_json::to_value(parsed).map_err(failure)
}

fn failure<Error>(error: Error) -> Box<HelperError>
where
    Error: std::fmt::Display,
{
    Box::new(HelperError {
        class: "B8021".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
