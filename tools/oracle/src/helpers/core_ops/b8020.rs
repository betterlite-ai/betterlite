use std::collections::BTreeMap;

use betterlite_core::api::search_providers::exa_ai::reply::decode_exa_ai_search;
use betterlite_core::api::search_providers::exa_ai::{ExaAiSearchConfig, ExaAiSearchSettings};
use betterlite_core::api::search_providers::serper::reply::decode_serper_search;
use betterlite_core::api::search_providers::serper::{SerperSearchConfig, SerperSearchSettings};
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::provider::{SearchProvider, SearchQuery};
use betterlite_openai::search::records::SearchResponse;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

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

trait UnderTest: SearchProvider {
    fn headers_for(
        &self,
        headers: Headers,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> Result<Headers, String>;

    fn settings_value(&self) -> Value;

    fn decode_reply(&self, body: &str) -> Result<SearchResponse, String>;
}

impl UnderTest for ExaAiSearchConfig {
    fn headers_for(
        &self,
        headers: Headers,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> Result<Headers, String> {
        self.validated_headers(headers, api_key, api_base)
            .map_err(|error| error.to_string())
    }

    fn settings_value(&self) -> Value {
        json!({
            "api_key": self.settings.api_key.as_ref().map(ExposeSecret::expose_secret),
            "api_base": self.settings.api_base,
        })
    }

    fn decode_reply(&self, body: &str) -> Result<SearchResponse, String> {
        decode_exa_ai_search(body.as_bytes()).map_err(|error| error.to_string())
    }
}

impl UnderTest for SerperSearchConfig {
    fn headers_for(
        &self,
        headers: Headers,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> Result<Headers, String> {
        self.validated_headers(headers, api_key, api_base)
            .map_err(|error| error.to_string())
    }

    fn settings_value(&self) -> Value {
        json!({
            "api_key": self.settings.api_key.as_ref().map(ExposeSecret::expose_secret),
            "api_base": self.settings.api_base,
        })
    }

    fn decode_reply(&self, body: &str) -> Result<SearchResponse, String> {
        decode_serper_search(body.as_bytes()).map_err(|error| error.to_string())
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let mut outcomes = Vec::with_capacity(input.calls.len());
        for call in &input.calls {
            let result = match call.provider {
                Provider::ExaAi => {
                    let config = ExaAiSearchConfig {
                        settings: ExaAiSearchSettings::load(&input.environment),
                    };
                    run_operation(&config, &call.call)
                }
                Provider::Serper => {
                    let config = SerperSearchConfig {
                        settings: SerperSearchSettings::load(&input.environment),
                    };
                    run_operation(&config, &call.call)
                }
            };
            outcomes.push(outcome(result));
        }
        serde_json::to_value(outcomes).map_err(|error| failure(&error))
    }
}

fn outcome(result: Result<Value, String>) -> Value {
    result.map_or_else(
        |_| json!({"ok": false}),
        |value| json!({"ok": true, "value": value}),
    )
}

fn run_operation<P>(config: &P, operation: &Operation) -> Result<Value, String>
where
    P: UnderTest,
{
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
            serde_json::to_value(validated).map_err(|error| error.to_string())
        }
        Operation::CompleteUrl { api_base } => config
            .complete_url(api_base.as_deref(), &Object::new(), None)
            .map(Value::String)
            .map_err(|error| error.to_string()),
        Operation::Request {
            query,
            optional_params,
        } => {
            let body = config
                .transform_search_request(query, optional_params)
                .map_err(|error| error.to_string())?;
            serde_json::to_value(body).map_err(|error| error.to_string())
        }
        Operation::Reply { body } => {
            let response = config.decode_reply(body)?;
            serde_json::to_value(response).map_err(|error| error.to_string())
        }
    }
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
