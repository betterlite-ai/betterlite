use std::collections::BTreeMap;

use betterlite_core::api::search_providers::dataforseo::reply::decode_dataforseo_search;
use betterlite_core::api::search_providers::dataforseo::{
    DataForSeoSearchConfig, DataForSeoSearchError, DataForSeoSearchSettings,
};
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::provider::{SearchProvider as _, SearchQuery};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    environment: BTreeMap<String, String>,
    calls: Vec<Operation>,
}

#[derive(Debug, Deserialize)]
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
enum DataForSeoHelperError {
    #[error(transparent)]
    Search(#[from] DataForSeoSearchError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        let config = DataForSeoSearchConfig {
            settings: DataForSeoSearchSettings::load(&input.environment),
        };
        let mut outcomes = Vec::with_capacity(input.calls.len());
        for operation in &input.calls {
            outcomes.push(outcome(run_operation(&config, operation)));
        }
        serde_json::to_value(outcomes).map_err(|error| failure(&error))
    }
}

fn run_operation(
    config: &DataForSeoSearchConfig,
    operation: &Operation,
) -> Result<Value, DataForSeoHelperError> {
    match operation {
        Operation::Settings => Ok(settings_value(config)),
        Operation::ValidateEnvironment {
            api_key,
            api_base,
            headers,
        } => {
            let key = api_key.as_deref().map(SecretString::from);
            let incoming: Headers = headers.clone().into_iter().collect();
            let validated =
                config.validated_headers(incoming, key.as_ref(), api_base.as_deref())?;
            Ok(serde_json::to_value(validated)?)
        }
        Operation::CompleteUrl { api_base } => {
            let url = config.complete_url(api_base.as_deref(), &Object::new(), None)?;
            Ok(Value::String(url))
        }
        Operation::Request {
            query,
            optional_params,
        } => Ok(serde_json::to_value(
            config.transform_search_request(query, optional_params)?,
        )?),
        Operation::Reply { body } => Ok(serde_json::to_value(decode_dataforseo_search(
            body.as_bytes(),
        )?)?),
    }
}

fn settings_value(config: &DataForSeoSearchConfig) -> Value {
    let settings = &config.settings;
    json!({
        "login": settings.login.as_ref().map(ExposeSecret::expose_secret),
        "password": settings.password.as_ref().map(ExposeSecret::expose_secret),
        "api_base": settings.api_base,
        "location_code": settings.location_code,
    })
}

fn outcome(result: Result<Value, DataForSeoHelperError>) -> Value {
    result.map_or_else(
        |_| json!({"ok": false}),
        |value| json!({"ok": true, "value": value}),
    )
}

fn failure(error: &serde_json::Error) -> Box<HelperError> {
    Box::new(HelperError {
        class: "DataForSeoInput".to_owned(),
        message: error.to_string(),
        status: None,
        error_type: None,
        param: None,
        code: None,
    })
}
