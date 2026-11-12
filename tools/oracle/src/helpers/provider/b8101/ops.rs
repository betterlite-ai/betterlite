use betterlite_provider::langflow_chat::config::{
    SUPPORTED_OPENAI_PARAMS, complete_url, provider_info, sign_request, validate_environment,
};
use betterlite_provider::langflow_chat::error::LangFlowError;
use betterlite_provider::langflow_chat::reply::{ReplyInput, transform_response};
use betterlite_provider::langflow_chat::request::{RequestInput, transform_request};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};

use super::fixture::{OneTokenCounter, secrets};
use super::script::{FlowInput, ReplyCase, RequestCase, SignCase, UrlCase, ValidateCase};
use crate::helpers::{HelperError, HelperResult};

pub(super) fn flow(input: &FlowInput) -> HelperResult {
    let secrets = secrets();
    let key = input.provider.api_key.as_deref().map(SecretString::from);
    let info = provider_info(
        input.provider.api_base.as_deref(),
        key.as_ref(),
        &secrets,
        &input.provider.environment,
    )
    .map_err(failure)?;
    Ok(json!({
        "supported": SUPPORTED_OPENAI_PARAMS,
        "provider": {
            "api_base": info.api_base,
            "api_key": info.api_key.as_ref().map(|key| key.expose_secret().clone()),
        },
        "urls": input.urls.iter().map(url_value).collect::<Vec<Value>>(),
        "validated": input
            .validate
            .iter()
            .map(validate_value)
            .collect::<Vec<Value>>(),
    }))
}

fn url_value(case: &UrlCase) -> Value {
    match complete_url(case.api_base.as_deref(), &case.model, &case.options) {
        Ok(url) => json!({"ok": true, "url": url}),
        Err(error) => json!({"ok": false, "status": error.status_code()}),
    }
}

fn validate_value(case: &ValidateCase) -> Value {
    let mut headers = case.headers.clone();
    let key = case.api_key.as_deref().map(SecretString::from);
    validate_environment(&mut headers, key.as_ref());
    json!(headers)
}

pub(super) fn requests(cases: &[RequestCase], signatures: &[SignCase]) -> Value {
    let bodies: Vec<Value> = cases
        .iter()
        .map(|case| {
            let input = RequestInput {
                messages: &case.messages,
                optional_params: &case.options,
            };
            match transform_request(&input) {
                Ok(body) => json!({"ok": true, "body": body}),
                Err(error) => json!({"ok": false, "status": error.status_code()}),
            }
        })
        .collect();
    let signed: Vec<Value> = signatures
        .iter()
        .map(
            |case| match sign_request(case.headers.clone(), &case.request_data) {
                Ok(headers) => json!({"ok": true, "headers": headers, "body": false}),
                Err(error) => json!({"ok": false, "status": error.status_code()}),
            },
        )
        .collect();
    json!({"bodies": bodies, "signatures": signed})
}

pub(super) fn replies(cases: &[ReplyCase]) -> HelperResult {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|source| {
            Box::new(HelperError {
                class: String::from("Runtime"),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: source.to_string(),
            })
        })?;
    let counter = OneTokenCounter;
    let mut found = Vec::with_capacity(cases.len());
    for case in cases {
        let input = ReplyInput {
            model: &case.model,
            status: case.status,
            body: case.body.as_bytes(),
            messages: &case.messages,
        };
        match runtime.block_on(transform_response(&input, &counter)) {
            Ok(reply) => found.push(json!({
                "ok": true,
                "model": reply.model,
                "index": reply.choice.index,
                "finish_reason": reply.choice.finish_reason,
                "role": reply.choice.message.role,
                "content": reply.choice.message.content,
            })),
            Err(error) => found.push(json!({"ok": false, "status": error.status_code()})),
        }
    }
    Ok(json!({"replies": found}))
}

fn failure(error: LangFlowError) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("LangFlowError"),
        status: error.status_code(),
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
