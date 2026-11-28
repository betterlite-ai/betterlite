use std::collections::BTreeSet;

use betterlite_bedrock::bedrock_embedding_io::error::EmbeddingIoError;
use betterlite_bedrock::bedrock_embedding_io::transport::{EmbeddingCall, post_embedding};
use betterlite_bedrock::bedrock_settings::{BedrockError, ErrorHeaders};
use betterlite_core::api::vector_store_http_write::service::SuppliedClient;
use betterlite_prompts::arguments::json::Object;
use serde_json::{Value, json};

use super::failure::Failure;
use super::input::HttpRequest;
use super::runtime;
use super::signed;
use super::transport::{self, Factory};

pub(crate) fn provider_json_response(case: &HttpRequest) -> Result<Value, Failure> {
    let headers = signed::pairs(&case.headers);
    let names = transport::case_headers(case);
    let supplied = transport::supplied_client(case);
    let factory = Factory::new(case.reply.clone());
    let embedding_call = call(case, &headers);
    let supplied_view = supplied.as_ref().map(|(kind, client)| SuppliedClient {
        kind: *kind,
        transport: client,
    });
    let value = runtime::block_on(post_embedding(&embedding_call, supplied_view, &factory))??;
    let supplied_requests = match &supplied {
        Some((_, client)) => transport::request_values(&client.recorded(), &names)?,
        None => Vec::new(),
    };
    Ok(json!({
        "factory": factory.calls(),
        "supplied_requests": supplied_requests,
        "factory_requests": transport::request_values(&factory.recorded(), &names)?,
        "value": value,
    }))
}

pub(crate) fn http_error(case: &HttpRequest) -> Result<Value, Failure> {
    let headers = signed::pairs(&case.headers);
    let names = transport::reply_headers(case);
    let supplied = transport::supplied_client(case);
    let factory = Factory::new(case.reply.clone());
    let embedding_call = call(case, &headers);
    let supplied_view = supplied.as_ref().map(|(kind, client)| SuppliedClient {
        kind: *kind,
        transport: client,
    });
    match runtime::block_on(post_embedding(&embedding_call, supplied_view, &factory))? {
        Ok(value) => Ok(json!({"kind": "ok", "value": value})),
        Err(EmbeddingIoError::Provider { error, .. }) => provider_detail(&error, &names),
        Err(error) => Err(Failure::Embedding(error)),
    }
}

fn call<'source>(
    case: &'source HttpRequest,
    headers: &'source [(String, String)],
) -> EmbeddingCall<'source> {
    let timeout = transport::case_timeout(case);
    EmbeddingCall {
        asynchronous: case.asynchronous,
        url: &case.url,
        headers,
        data: &case.data,
        timeout,
    }
}

fn provider_detail(error: &BedrockError, names: &BTreeSet<String>) -> Result<Value, Failure> {
    let provider = &error.error;
    let headers = match error.headers.as_ref() {
        None => Value::Null,
        Some(ErrorHeaders::Http(pairs)) => serde_json::to_value(filtered_headers(pairs, names)?)?,
        Some(ErrorHeaders::Mapping(entries)) => serde_json::to_value(entries)?,
    };
    Ok(json!({
        "kind": "error",
        "status_code": provider.status_code,
        "status_code_is_synthesized": provider.status_code_is_synthesized,
        "request": {"method": provider.request.method, "url": provider.request.url},
        "response": {
            "status_code": provider.response.status_code,
            "request": {
                "method": provider.response.request.method,
                "url": provider.response.request.url,
            },
        },
        "headers": headers,
        "body": provider.body,
        "response_headers": filtered_headers(&error.response_headers, names)?,
    }))
}

fn filtered_headers(
    pairs: &[(String, String)],
    names: &BTreeSet<String>,
) -> Result<Object, Failure> {
    let mut headers = Object::new();
    for (name, value) in pairs {
        let lowered = name.to_ascii_lowercase();
        if names.contains(&lowered) {
            headers.insert(lowered, serde_json::from_value(json!(value))?);
        }
    }
    Ok(headers)
}
