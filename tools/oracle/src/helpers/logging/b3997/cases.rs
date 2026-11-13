use std::sync::{Arc, PoisonError};

use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_logging::gcs_client::gcs_storage_auth::RequestHeaders;
use betterlite_logging::gcs_client::gcs_storage_auth::config_resolution_behaviors::logging_config;
use betterlite_logging::gcs_client::gcs_storage_auth::download_behaviors::download_gcs_object;
use betterlite_logging::gcs_client::gcs_storage_auth::error::GcsClientError;
use betterlite_logging::gcs_client::gcs_storage_auth::header_construction_behaviors::construct_request_headers;
use betterlite_logging::gcs_client::gcs_storage_auth::sync_header_behaviors::sync_construct_request_headers;
use betterlite_logging::gcs_client::gcs_storage_auth::upload_behaviors::log_json_data_on_gcs;
use betterlite_logging::gcs_client::gcs_storage_auth::vertex_caching_behaviors::get_or_create_vertex_instance;
use betterlite_logging::gcs_mock::request::GcsRequest;
use betterlite_logging::types::gcs_storage_auth::type_definitions::{
    GcsLoggingKwargs, GcsLoggingPayload,
};
use betterlite_models::decisions::Json;
use secrecy::SecretString;
use serde_json::{Value, json};

use super::output::{
    body_json, ensure_json, headers_json, json_value, parse_or_text, project_payload, secret_text,
    token_json,
};
use super::recording::{FakeAuth, RecordingClient};
use super::{Session, bucket_name, failure, open, required_text};
use crate::helpers::HelperError;

pub(super) async fn initialize_case(input: &super::Input) -> crate::helpers::HelperResult {
    let session = open(input)?;
    let mut downloaded = Value::Null;
    if let Some(object_name) = &input.object_name {
        let body =
            download_gcs_object(&session.base, object_name, &GcsLoggingKwargs::default()).await;
        downloaded = download_json(&session, body.as_deref());
    }
    Ok(json!({
        "is_mock_mode": session.base.is_mock_mode,
        "path_service_account": secret_text(session.base.path_service_account.as_ref()),
        "download": downloaded,
    }))
}

pub(super) async fn async_headers_case(input: &super::Input) -> crate::helpers::HelperResult {
    let session = open(input)?;
    let credentials = input.credentials.clone().map(SecretString::from);
    let headers = construct_request_headers(
        &session.base,
        bucket_name(input),
        credentials.as_ref(),
        None,
    )
    .await
    .map_err(|error| failure("ValueError", error))?;
    Ok(header_json(&session, &headers))
}

pub(super) fn sync_headers_case(input: &super::Input) -> crate::helpers::HelperResult {
    let session = open(input)?;
    let path = session.base.path_service_account.clone();
    let headers = sync_construct_request_headers(&session.base, bucket_name(input), path.as_ref())
        .map_err(|error| failure("ValueError", error))?;
    Ok(header_json(&session, &headers))
}

pub(super) async fn upload_case(input: &super::Input) -> crate::helpers::HelperResult {
    let session = open(input)?;
    let bucket = required_text(input.bucket.as_deref(), "bucket")?;
    let object_name = required_text(input.object_name.as_deref(), "object_name")?;
    let payload = logging_payload(input.payload.as_ref())?;
    let headers = RequestHeaders::new();
    let response = log_json_data_on_gcs(&session.base, &headers, bucket, object_name, &payload)
        .await
        .map_err(upload_failure)?;
    Ok(json!({
        "response": json_value(&response),
        "request": last_request(&session).map(request_json),
    }))
}

pub(super) async fn logging_config_case(input: &super::Input) -> crate::helpers::HelperResult {
    let session = open(input)?;
    let kwargs = GcsLoggingKwargs {
        dynamic_params: dynamic_params(input.dynamic.as_ref())?,
    };
    let config = logging_config(&session.base, &kwargs)
        .await
        .map_err(|error| failure("ValueError", error))?;
    Ok(json!({
        "config": {
            "bucket_name": config.bucket_name,
            "path_service_account": secret_text(config.path_service_account.as_ref()),
        },
        "cached_keys": cache_keys(&session),
    }))
}

pub(super) async fn vertex_instance_case(input: &super::Input) -> crate::helpers::HelperResult {
    let session = open(input)?;
    let credentials = input.credentials.clone().map(SecretString::from);
    let mut instances = Vec::new();
    for requested in [None, None, credentials.as_ref()] {
        let instance = get_or_create_vertex_instance(&session.base, requested)
            .await
            .map_err(|error| failure("ValueError", error))?;
        instances.push(instance);
    }
    let (first, second, other) = (&instances[0], &instances[1], &instances[2]);
    Ok(json!({
        "instances": {
            "same_instance": Arc::ptr_eq(first, second),
            "distinct_credentials": !Arc::ptr_eq(first, other),
        },
        "cached_keys": cache_keys(&session),
    }))
}

fn cache_keys(session: &Session) -> Value {
    let instances = session
        .base
        .vertex_instances
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let mut keys: Vec<String> = instances
        .keys()
        .map(|key| secrecy::ExposeSecret::expose_secret(&key.0).to_owned())
        .collect();
    keys.sort();
    json!(keys)
}

fn header_json(session: &Session, headers: &RequestHeaders) -> Value {
    let ensure = session
        .auth
        .ensure_calls
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .last()
        .map(ensure_json);
    let token = session
        .auth
        .token_calls
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .last()
        .map(token_json);
    json!({
        "headers": headers_json(headers),
        "ensure": ensure,
        "token": token,
    })
}

fn download_json(session: &Session, body: Option<&[u8]>) -> Value {
    json!({
        "body": body_json(body),
        "request": last_request(session).map(request_json),
    })
}

fn last_request(session: &Session) -> Option<GcsRequest> {
    session
        .sent
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .last()
        .cloned()
}

fn request_json(request: &GcsRequest) -> Value {
    json!({
        "method": request.method().to_string(),
        "url": request.url(),
        "authorization": authorization(request),
        "body": request_body(request),
    })
}

fn authorization(request: &GcsRequest) -> Value {
    let headers = match request {
        GcsRequest::Get(read) => &read.headers,
        GcsRequest::Post(write) | GcsRequest::Delete(write) => &write.headers,
    };
    headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .map_or(Value::Null, |text| json!(text))
}

fn request_body(request: &GcsRequest) -> Value {
    match request {
        GcsRequest::Get(_) => Value::Null,
        GcsRequest::Post(write) | GcsRequest::Delete(write) => match &write.data {
            Some(Json::Text(text)) => project_payload(parse_or_text(text)),
            Some(other) => json_value(other),
            None => Value::Null,
        },
    }
}

fn dynamic_params(value: Option<&Value>) -> Result<Option<Json>, Box<crate::helpers::HelperError>> {
    match value {
        None => Ok(None),
        Some(value) => serde_json::from_value::<Json>(value.clone())
            .map(Some)
            .map_err(|error| failure("ValueError", error)),
    }
}

fn logging_payload(
    value: Option<&Value>,
) -> Result<GcsLoggingPayload, Box<crate::helpers::HelperError>> {
    match value {
        Some(Value::String(text)) => Ok(GcsLoggingPayload::Text(text.clone())),
        Some(value) => {
            let standard: StandardLoggingPayload = serde_json::from_value(value.clone())
                .map_err(|error| failure("ValueError", error))?;
            Ok(GcsLoggingPayload::Standard(Box::new(standard)))
        }
        None => Err(failure("ValueError", "payload is required")),
    }
}

fn upload_failure(error: GcsClientError<FakeAuth, RecordingClient>) -> Box<HelperError> {
    let mut helper = failure("MaskedHTTPStatusError", &error);
    if let GcsClientError::Status { status } = error {
        helper.status = Some(status);
    }
    helper
}
