use std::collections::BTreeMap;

use betterlite_openai::chat_content::RefuseDownloads;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_provider::ragflow_chat::env::{
    CredentialsInput, Globals, STORED_MODEL_FIELD, provider_info, validate_environment,
};
use betterlite_provider::ragflow_chat::error::RagflowError;
use betterlite_provider::ragflow_chat::request::{RequestInput, transform_request};
use betterlite_provider::ragflow_chat::url::complete_url;
use secrecy::ExposeSecret;
use serde_json::{Value, json};

use super::script::{HeadersCase, ProviderCase, RequestCase, Sdk, UrlCase};
use super::stubs::secret_lookup;

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

pub(super) fn provider(
    environment: &BTreeMap<String, String>,
    sdk: &Sdk,
    cases: &[ProviderCase],
) -> Value {
    let secrets = secret_lookup();
    let mut found = Vec::with_capacity(cases.len());
    for case in cases {
        let input = CredentialsInput {
            model: &case.model,
            api_base: case.api_base.as_deref(),
            api_key: case.api_key.as_ref(),
            globals: Globals {
                api_base: sdk.api_base.as_deref(),
                api_key: sdk.api_key.as_ref(),
            },
            secrets: &secrets,
            environment,
        };
        found.push(match provider_info(&input) {
            Ok(info) => json!({
                "ok": true,
                "base": info.api_base,
                "key": info.api_key.as_ref().map(ExposeSecret::expose_secret),
            }),
            Err(error) => error_value(&error),
        });
    }
    json!({"probes": found})
}

pub(super) fn url(environment: &BTreeMap<String, String>, sdk: &Sdk, cases: &[UrlCase]) -> Value {
    let secrets = secret_lookup();
    let mut found = Vec::with_capacity(cases.len());
    for case in cases {
        let input = CredentialsInput {
            model: &case.model,
            api_base: case.api_base.as_deref(),
            api_key: None,
            globals: Globals {
                api_base: sdk.api_base.as_deref(),
                api_key: None,
            },
            secrets: &secrets,
            environment,
        };
        found.push(match complete_url(&input) {
            Ok(url) => json!({"ok": true, "url": url}),
            Err(error) => error_value(&error),
        });
    }
    json!({"probes": found})
}

pub(super) fn headers(
    environment: &BTreeMap<String, String>,
    sdk: &Sdk,
    cases: &[HeadersCase],
) -> Value {
    let secrets = secret_lookup();
    let mut found = Vec::with_capacity(cases.len());
    for case in cases {
        let input = CredentialsInput {
            model: &case.model,
            api_base: None,
            api_key: case.api_key.as_ref(),
            globals: Globals {
                api_base: None,
                api_key: sdk.api_key.as_ref(),
            },
            secrets: &secrets,
            environment,
        };
        let mut merged = case.headers.clone();
        let mut params = Object::new();
        if let Err(error) = validate_environment(&input, &mut merged, &mut params) {
            found.push(error_value(&error));
            continue;
        }
        found.push(json!({
            "headers": merged,
            "stored": params.get(STORED_MODEL_FIELD),
        }));
    }
    json!({"probes": found})
}

pub(super) fn requests(
    environment: &BTreeMap<String, String>,
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
            environment,
            downloads: &RefuseDownloads,
        })
        .map_err(|source| helper_error("RequestTransform", source.to_string()))?;
        found.push(json!({"model": body.get("model")}));
    }
    Ok(json!({"probes": found}))
}
