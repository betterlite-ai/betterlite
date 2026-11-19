use std::collections::BTreeMap;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_provider::ragflow_chat::env::{
    CredentialsInput, Globals, STORED_MODEL_FIELD, provider_info, validate_environment,
};
use betterlite_provider::ragflow_chat::error::RagflowError;
use betterlite_provider::ragflow_chat::request::{RequestInput, transform_request};
use betterlite_provider::ragflow_chat::url::complete_url;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use secrecy::ExposeSecret;
use serde_json::{Value, json};

use super::script::{ProbeCase, RequestCase, Sdk};
use super::stubs::secret_lookup;

type Environment = BTreeMap<String, String>;

#[derive(Clone, Copy)]
pub(super) enum Operation {
    Provider,
    Url,
    Headers,
}

fn error_value(error: &RagflowError) -> Value {
    json!({
        "ok": false,
        "error": {
            "class": "ValueError",
            "message": error.to_string(),
        },
    })
}

fn helper_error(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn credentials<'request>(
    case: &'request ProbeCase,
    secrets: &'request SecretLookup,
    environment: &'request Environment,
    sdk: &'request Sdk,
) -> CredentialsInput<'request> {
    CredentialsInput {
        model: case.model.as_str(),
        api_base: case.api_base.as_deref(),
        api_key: case.api_key.as_ref(),
        globals: Globals {
            api_base: sdk.api_base.as_deref(),
            api_key: sdk.api_key.as_ref(),
        },
        secrets,
        environment,
    }
}

fn headers_value(input: &CredentialsInput<'_>, mut headers: BTreeMap<String, String>) -> Value {
    let mut params = Object::new();
    validate_environment(input, &mut headers, &mut params).map_or_else(
        |error| error_value(&error),
        |()| {
            json!({
                "headers": headers,
                "stored": params.get(STORED_MODEL_FIELD),
            })
        },
    )
}

fn probe(
    operation: Operation,
    case: &ProbeCase,
    secrets: &SecretLookup,
    environment: &Environment,
    sdk: &Sdk,
) -> Value {
    let input = credentials(case, secrets, environment, sdk);
    match operation {
        Operation::Provider => provider_info(&input).map_or_else(
            |error| error_value(&error),
            |info| {
                json!({
                    "ok": true,
                    "base": info.api_base,
                    "key": info.api_key.as_ref().map(ExposeSecret::expose_secret),
                })
            },
        ),
        Operation::Url => complete_url(&input).map_or_else(
            |error| error_value(&error),
            |url| json!({"ok": true, "url": url}),
        ),
        Operation::Headers => headers_value(&input, case.headers.clone()),
    }
}

pub(super) fn probes(
    operation: Operation,
    environment: &Environment,
    sdk: &Sdk,
    cases: &[ProbeCase],
) -> Value {
    let secrets = secret_lookup();
    let found: Vec<Value> = cases
        .iter()
        .map(|case| probe(operation, case, &secrets, environment, sdk))
        .collect();
    json!({"probes": found})
}

pub(super) fn requests(
    _environment: &Environment,
    _sdk: &Sdk,
    cases: &[RequestCase],
) -> HelperResult {
    let mut found = Vec::with_capacity(cases.len());
    for case in cases {
        let mut params = Object::new();
        if let Some(stored) = &case.stored {
            params.insert(STORED_MODEL_FIELD.to_owned(), Json::Text(stored.clone()));
        }
        let body = transform_request(&RequestInput {
            model: &case.model,
            messages: &case.messages,
            optional_params: &case.options,
            litellm_params: &params,
        })
        .map_err(|source| helper_error("RequestTransform", source.to_string()))?;
        found.push(
            serde_json::to_value(&body)
                .map_err(|source| helper_error("RequestTransform", source.to_string()))?,
        );
    }
    Ok(json!({"probes": found}))
}
