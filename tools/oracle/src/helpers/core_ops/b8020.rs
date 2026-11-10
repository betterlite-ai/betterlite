use std::collections::BTreeMap;

use betterlite_core::api::search_providers::exa_ai::reply::decode_exa_ai_search;
use betterlite_core::api::search_providers::exa_ai::{
    ExaAiSearchConfig, ExaAiSearchError, ExaAiSearchSettings,
};
use betterlite_core::api::search_providers::serper::reply::decode_serper_search;
use betterlite_core::api::search_providers::serper::{
    SerperSearchConfig, SerperSearchError, SerperSearchSettings,
};
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::provider::{SearchProvider as _, SearchQuery};
use betterlite_openai::search::records::SearchResponse;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    environment: BTreeMap<String, String>,
    calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    provider: Provider,
    call: Operation,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum Provider {
    ExaAi,
    Serper,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Operation {
    Settings,
    ValidateEnvironment {
        #[serde(default)]
        api_key: Option<String>,
        #[serde(default)]
        api_base: Option<String>,
        #[serde(default)]
        headers: BTreeMap<String, String>,
    },
    CompleteUrl {
        #[serde(default)]
        api_base: Option<String>,
    },
    Request {
        query: SearchQuery,
        #[serde(default)]
        optional_params: Object,
    },
    Reply {
        body: String,
    },
}

#[derive(Debug, Error)]
enum B8020HelperError {
    #[error(transparent)]
    Exa(#[from] ExaAiSearchError),
    #[error(transparent)]
    Serper(#[from] SerperSearchError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

enum Config {
    Exa(ExaAiSearchConfig),
    Serper(SerperSearchConfig),
}

impl Config {
    fn headers_for(
        &self,
        headers: Headers,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> Result<Headers, B8020HelperError> {
        match self {
            Self::Exa(config) => Ok(config.validated_headers(headers, api_key, api_base)?),
            Self::Serper(config) => Ok(config.validated_headers(headers, api_key, api_base)?),
        }
    }

    fn settings_value(&self) -> Value {
        match self {
            Self::Exa(config) => {
                settings_json(config.settings.api_key.as_ref(), &config.settings.api_base)
            }
            Self::Serper(config) => {
                settings_json(config.settings.api_key.as_ref(), &config.settings.api_base)
            }
        }
    }

    fn decode_reply(&self, body: &str) -> Result<SearchResponse, B8020HelperError> {
        match self {
            Self::Exa(_) => Ok(decode_exa_ai_search(body.as_bytes())?),
            Self::Serper(_) => Ok(decode_serper_search(body.as_bytes())?),
        }
    }

    fn complete_url(&self, api_base: Option<&str>) -> Result<String, B8020HelperError> {
        match self {
            Self::Exa(config) => Ok(config.complete_url(api_base, &Object::new(), None)?),
            Self::Serper(config) => Ok(config.complete_url(api_base, &Object::new(), None)?),
        }
    }

    fn request(&self, query: &SearchQuery, options: &Object) -> Result<Value, B8020HelperError> {
        match self {
            Self::Exa(config) => Ok(serde_json::to_value(
                config.transform_search_request(query, options)?,
            )?),
            Self::Serper(config) => Ok(serde_json::to_value(
                config.transform_search_request(query, options)?,
            )?),
        }
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let mut outcomes = Vec::with_capacity(input.calls.len());
        for call in &input.calls {
            let config = match call.provider {
                Provider::ExaAi => Config::Exa(ExaAiSearchConfig {
                    settings: ExaAiSearchSettings::load(&input.environment),
                }),
                Provider::Serper => Config::Serper(SerperSearchConfig {
                    settings: SerperSearchSettings::load(&input.environment),
                }),
            };
            outcomes.push(outcome(run_operation(&config, &call.call)));
        }
        serde_json::to_value(outcomes).map_err(|error| failure(&error))
    }
}

fn outcome(result: Result<Value, B8020HelperError>) -> Value {
    result.map_or_else(
        |_| json!({"ok": false}),
        |value| json!({"ok": true, "value": value}),
    )
}

fn run_operation(config: &Config, operation: &Operation) -> Result<Value, B8020HelperError> {
    match operation {
        Operation::Settings => Ok(config.settings_value()),
        Operation::ValidateEnvironment {
            api_key,
            api_base,
            headers,
        } => {
            let key = api_key.as_deref().map(SecretString::from);
            let incoming: Headers = headers.clone().into_iter().collect();
            let validated = config.headers_for(incoming, key.as_ref(), api_base.as_deref())?;
            Ok(serde_json::to_value(validated)?)
        }
        Operation::CompleteUrl { api_base } => {
            config.complete_url(api_base.as_deref()).map(Value::String)
        }
        Operation::Request {
            query,
            optional_params,
        } => config.request(query, optional_params),
        Operation::Reply { body } => {
            let response = config.decode_reply(body)?;
            Ok(serde_json::to_value(response)?)
        }
    }
}

fn settings_json(api_key: Option<&SecretString>, api_base: &str) -> Value {
    json!({
        "api_key": api_key.map(ExposeSecret::expose_secret),
        "api_base": api_base,
    })
}

fn failure(error: &serde_json::Error) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: "B8020Input".to_owned(),
        message: error.to_string(),
        status: None,
        error_type: None,
        param: None,
        code: None,
    })
}
