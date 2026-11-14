use std::sync::{Arc, PoisonError};

use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_logging::gcs_client::gcs_storage_auth::RequestHeaders;
use betterlite_logging::gcs_client::gcs_storage_auth::VertexInstance;
use betterlite_logging::gcs_client::gcs_storage_auth::config_resolution_behaviors::logging_config;
use betterlite_logging::gcs_client::gcs_storage_auth::download_behaviors::download_gcs_object;
use betterlite_logging::gcs_client::gcs_storage_auth::error::GcsClientError;
use betterlite_logging::gcs_client::gcs_storage_auth::header_construction_behaviors::construct_request_headers;
use betterlite_logging::gcs_client::gcs_storage_auth::sync_header_behaviors::sync_construct_request_headers;
use betterlite_logging::gcs_client::gcs_storage_auth::upload_behaviors::log_json_data_on_gcs;
use betterlite_logging::gcs_client::gcs_storage_auth::vertex_caching_behaviors::get_or_create_vertex_instance;
use betterlite_logging::gcs_mock::request::GcsRequest;
use betterlite_logging::types::gcs_storage_auth::type_definitions::{
    GcsLoggingConfig, GcsLoggingKwargs, GcsLoggingPayload,
};
use betterlite_models::decisions::Json;
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};

use super::output::{body_json, ensure_json, headers_json, project_payload, token_json};
use super::recording::{FakeAuth, RecordingClient};
use super::{Session, failure, open};
use crate::helpers::HelperError;

pub(super) enum HeaderMode {
    Async,
    Sync,
}

pub(super) async fn initialize_case(input: &super::Input) -> crate::helpers::HelperResult {
    let session = open(input)?;
    let download = download_value(&session, input.object_name.as_deref()).await;
    Ok(initialize_json(&session, download))
}

async fn download_value(session: &Session, object_name: Option<&str>) -> Value {
    let Some(object_name) = object_name else {
        return Value::Null;
    };
    let body = download_gcs_object(&session.base, object_name, &GcsLoggingKwargs::default()).await;
    download_json(session, body.as_deref())
}

fn initialize_json(session: &Session, download: Value) -> Value {
    json!({
        "is_mock_mode": session.base.is_mock_mode,
        "path_service_account": json!(session.base.path_service_account.as_ref().map(ExposeSecret::expose_secret)),
        "download": download,
    })
}

pub(super) async fn headers_case(
    input: &super::Input,
    mode: HeaderMode,
) -> crate::helpers::HelperResult {
    let session = open(input)?;
    let headers = request_headers(&session, input, mode).await?;
    Ok(header_json(&session, &headers))
}

async fn request_headers(
    session: &Session,
    input: &super::Input,
    mode: HeaderMode,
) -> Result<RequestHeaders, Box<HelperError>> {
    let credentials = input.credentials.clone().map(SecretString::from);
    let bucket = input.bucket.as_deref().unwrap_or("bucket");
    match mode {
        HeaderMode::Async => {
            construct_request_headers(&session.base, bucket, credentials.as_ref(), None).await
        }
        HeaderMode::Sync => sync_construct_request_headers(
            &session.base,
            bucket,
            session.base.path_service_account.as_ref(),
        ),
    }
    .map_err(|error| failure("ValueError", error))
}

pub(super) async fn upload_case(input: &super::Input) -> crate::helpers::HelperResult {
    let session = open(input)?;
    let response = send_upload(&session, input).await?;
    Ok(upload_json(&session, &response))
}

async fn send_upload(session: &Session, input: &super::Input) -> Result<Json, Box<HelperError>> {
    let bucket = input
        .bucket
        .as_deref()
        .ok_or_else(|| failure("ValueError", "bucket"))?;
    let object_name = input
        .object_name
        .as_deref()
        .ok_or_else(|| failure("ValueError", "object_name"))?;
    let payload = logging_payload(input.payload.as_ref())?;
    log_json_data_on_gcs(
        &session.base,
        &RequestHeaders::new(),
        bucket,
        object_name,
        &payload,
    )
    .await
    .map_err(upload_failure)
}

fn upload_json(session: &Session, response: &Json) -> Value {
    json!({
        "response": serde_json::to_value(response).unwrap_or(Value::Null),
        "request": last_request(session).map(request_json),
    })
}

pub(super) async fn logging_config_case(input: &super::Input) -> crate::helpers::HelperResult {
    let session = open(input)?;
    let config = logging_config_for(&session, input).await?;
    Ok(config_json(&session, &config))
}

async fn logging_config_for(
    session: &Session,
    input: &super::Input,
) -> Result<GcsLoggingConfig<FakeAuth>, Box<HelperError>> {
    let kwargs = GcsLoggingKwargs {
        dynamic_params: dynamic_params(input.dynamic.as_ref())?,
    };
    logging_config(&session.base, &kwargs)
        .await
        .map_err(|error| failure("ValueError", error))
}

fn dynamic_params(value: Option<&Value>) -> Result<Option<Json>, Box<HelperError>> {
    match value {
        None => Ok(None),
        Some(value) => serde_json::from_value::<Json>(value.clone())
            .map(Some)
            .map_err(|error| failure("ValueError", error)),
    }
}

fn config_json(session: &Session, config: &GcsLoggingConfig<FakeAuth>) -> Value {
    json!({
        "config": {
            "bucket_name": config.bucket_name,
            "path_service_account": json!(config.path_service_account.as_ref().map(ExposeSecret::expose_secret)),
        },
        "cached_keys": cache_keys(session),
    })
}

pub(super) async fn vertex_instance_case(input: &super::Input) -> crate::helpers::HelperResult {
    let session = open(input)?;
    let instances = cached_instances(&session, input).await?;
    let [first, second, other] = instances.as_slice() else {
        return Err(failure(
            "RuntimeError",
            "the vertex cache returned an unexpected number of instances",
        ));
    };
    Ok(instances_json(&session, first, second, other))
}

async fn cached_instances(
    session: &Session,
    input: &super::Input,
) -> Result<Vec<VertexInstance<FakeAuth>>, Box<HelperError>> {
    let credentials = input.credentials.clone().map(SecretString::from);
    let mut instances = Vec::new();
    for requested in [None, None, credentials.as_ref()] {
        let instance = get_or_create_vertex_instance(&session.base, requested)
            .await
            .map_err(|error| failure("ValueError", error))?;
        instances.push(instance);
    }
    Ok(instances)
}

fn instances_json(
    session: &Session,
    first: &VertexInstance<FakeAuth>,
    second: &VertexInstance<FakeAuth>,
    other: &VertexInstance<FakeAuth>,
) -> Value {
    json!({
        "instances": {
            "same_instance": Arc::ptr_eq(first, second),
            "distinct_credentials": !Arc::ptr_eq(first, other),
        },
        "cached_keys": cache_keys(session),
    })
}

fn cache_keys(session: &Session) -> Value {
    let instances = session
        .base
        .vertex_instances
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let mut keys: Vec<String> = instances
        .keys()
        .map(|key| key.0.expose_secret().to_owned())
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
    let headers = match request {
        GcsRequest::Get(read) => &read.headers,
        GcsRequest::Post(write) | GcsRequest::Delete(write) => &write.headers,
    };
    let authorization = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .map_or(Value::Null, |text| json!(text));
    let body = match request {
        GcsRequest::Get(_) => Value::Null,
        GcsRequest::Post(write) | GcsRequest::Delete(write) => match &write.data {
            Some(Json::Text(text)) => project_payload(
                serde_json::from_str(text).unwrap_or_else(|_error| Value::String(text.clone())),
            ),
            Some(other) => serde_json::to_value(other).unwrap_or(Value::Null),
            None => Value::Null,
        },
    };
    json!({
        "method": request.method().to_string(),
        "url": request.url(),
        "authorization": authorization,
        "body": body,
    })
}

fn logging_payload(
    value: Option<&Value>,
) -> Result<GcsLoggingPayload, Box<crate::helpers::HelperError>> {
    let Some(value) = value else {
        return Err(failure("ValueError", "payload is required"));
    };
    if let Value::String(text) = value {
        return Ok(GcsLoggingPayload::Text(text.clone()));
    }
    let standard: StandardLoggingPayload =
        serde_json::from_value(value.clone()).map_err(|error| failure("ValueError", error))?;
    Ok(GcsLoggingPayload::Standard(Box::new(standard)))
}

fn upload_failure(error: GcsClientError<FakeAuth, RecordingClient>) -> Box<HelperError> {
    let mut helper = failure("MaskedHTTPStatusError", &error);
    if let GcsClientError::Status { status } = error {
        helper.status = Some(status);
    }
    helper
}
