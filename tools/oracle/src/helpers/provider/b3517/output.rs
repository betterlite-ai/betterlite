use std::collections::BTreeMap;
use std::sync::PoisonError;
use std::time::Duration;

use betterlite_provider::opensandbox::helpers::{RequestTimeout, SandboxRequest};
use serde::Serialize;
use serde_json::Value;

use super::helper_error;
use super::transport::RecordingHttp;

#[derive(Serialize)]
pub(super) struct RequestOutput {
    method: String,
    url: String,
    headers: BTreeMap<String, String>,
    json: Option<Value>,
    timeout: TimeoutOutput,
}

#[derive(Serialize)]
struct TimeoutOutput {
    connect: Option<f64>,
    read: Option<f64>,
    write: Option<f64>,
    pool: Option<f64>,
}

pub(super) fn requests_output(
    http: &RecordingHttp,
) -> Result<Value, Box<crate::helpers::HelperError>> {
    let requests = http
        .requests
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let mut outputs = Vec::with_capacity(requests.len());
    for request in &requests {
        outputs.push(request_output(request)?);
    }
    serde_json::to_value(outputs).map_err(|error| helper_error("BodyEncoding", error))
}

pub(super) fn request_output(
    request: &SandboxRequest,
) -> Result<RequestOutput, Box<crate::helpers::HelperError>> {
    let mut headers = BTreeMap::new();
    for (name, value) in &request.headers {
        let text = value
            .to_str()
            .map_err(|error| helper_error("HeaderEncoding", error))?;
        headers.insert(name.as_str().to_owned(), text.to_owned());
    }
    let json = if request.body.is_empty() {
        None
    } else {
        Some(
            serde_json::from_slice(&request.body)
                .map_err(|error| helper_error("RequestBodyEncoding", error))?,
        )
    };
    Ok(RequestOutput {
        method: request.method.as_str().to_owned(),
        url: request.url.to_string(),
        headers,
        json,
        timeout: timeout_output(request.timeout),
    })
}

pub(super) fn single_request(
    http: &RecordingHttp,
) -> Result<SandboxRequest, Box<crate::helpers::HelperError>> {
    let requests = http.requests.lock().unwrap_or_else(PoisonError::into_inner);
    match (requests.first(), requests.len()) {
        (Some(request), 1) => Ok(request.clone()),
        _ => Err(helper_error("UnexpectedRequests", requests.len())),
    }
}

fn timeout_output(timeout: Option<RequestTimeout>) -> TimeoutOutput {
    let fields = timeout.unwrap_or(RequestTimeout {
        connect: None,
        read: None,
        write: None,
        pool: None,
    });
    TimeoutOutput {
        connect: fields.connect.as_ref().map(Duration::as_secs_f64),
        read: fields.read.as_ref().map(Duration::as_secs_f64),
        write: fields.write.as_ref().map(Duration::as_secs_f64),
        pool: fields.pool.as_ref().map(Duration::as_secs_f64),
    }
}
