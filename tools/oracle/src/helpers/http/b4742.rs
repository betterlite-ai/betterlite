mod connector;

use std::collections::BTreeMap;
use std::time::{Duration, UNIX_EPOCH};

use aws_smithy_async::rt::sleep::{SharedAsyncSleep, TokioSleep};
use aws_smithy_async::time::{SharedTimeSource, StaticTimeSource};
use aws_smithy_runtime_api::client::http::{SharedHttpConnector, http_client_fn};
use aws_types::os_shim_internal::Fs;
use betterlite_http::mcp_sigv4::error::McpSigV4Error;
use betterlite_http::mcp_sigv4::{AwsSources, McpSigV4Auth, McpSigV4Settings};
use bytes::Bytes;
use http::Request;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

use self::connector::{Recorder, Sts};
use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Settings {
    #[serde(rename = "aws_access_key_id")]
    access_key_id: Option<String>,
    #[serde(rename = "aws_secret_access_key")]
    secret_access_key: Option<String>,
    #[serde(rename = "aws_session_token")]
    session_token: Option<String>,
    #[serde(rename = "aws_region_name")]
    region_name: Option<String>,
    #[serde(rename = "aws_service_name")]
    service_name: Option<String>,
    #[serde(rename = "aws_role_name")]
    role_name: Option<String>,
    #[serde(rename = "aws_session_name")]
    session_name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Call {
    method: String,
    url: String,
    #[serde(default)]
    headers: Vec<(String, String)>,
    #[serde(default)]
    body: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    settings: Settings,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    now: u64,
    requests: Vec<Call>,
}

#[derive(Debug, thiserror::Error)]
enum Failure {
    #[error(transparent)]
    Sign(#[from] McpSigV4Error),
    #[error("the request is not valid: {0}")]
    Request(#[from] http::Error),
    #[error("the helper runtime failed: {0}")]
    Runtime(#[from] std::io::Error),
    #[error("the case time {0} is past the clock's range")]
    Clock(u64),
}

fn mcp_settings(settings: &Settings) -> McpSigV4Settings {
    let secret = |value: &Option<String>| value.clone().map(SecretString::from);
    McpSigV4Settings {
        aws_access_key_id: secret(&settings.access_key_id),
        aws_secret_access_key: secret(&settings.secret_access_key),
        aws_session_token: secret(&settings.session_token),
        aws_region_name: settings.region_name.clone(),
        aws_service_name: settings.service_name.clone(),
        aws_role_name: settings.role_name.clone(),
        aws_session_name: settings.session_name.clone(),
    }
}

fn sources(input: &Input, recorder: &Recorder) -> Result<AwsSources, Failure> {
    let sts = recorder.clone();
    Ok(AwsSources {
        environment: input
            .environment
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
        filesystem: Fs::from_slice(&[]),
        client: http_client_fn(move |_, _| SharedHttpConnector::new(Sts(sts.clone()))),
        clock: SharedTimeSource::new(StaticTimeSource::new(
            UNIX_EPOCH
                .checked_add(Duration::from_secs(input.now))
                .ok_or(Failure::Clock(input.now))?,
        )),
        sleep: SharedAsyncSleep::new(TokioSleep::new()),
        timeout: Duration::from_secs(5),
    })
}

async fn sign_requests(input: &Input, recorder: &Recorder) -> Result<Vec<Value>, Failure> {
    let auth = McpSigV4Auth::new(mcp_settings(&input.settings), sources(input, recorder)?).await?;
    let mut signed = Vec::new();
    for call in &input.requests {
        let mut builder = Request::builder()
            .method(call.method.as_str())
            .uri(&call.url);
        for (name, value) in &call.headers {
            builder = builder.header(name, value);
        }
        let request = builder.body(Bytes::from(call.body.clone()))?;
        let signed_request = auth.auth_flow(request).await?;
        let header = |name: &str| {
            signed_request
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
        };
        signed.push(json!({
            "authorization": header("authorization"),
            "x-amz-date": header("x-amz-date"),
            "x-amz-security-token": header("x-amz-security-token"),
        }));
    }
    Ok(signed)
}

fn failure(error: &Failure) -> Box<HelperError> {
    Box::new(HelperError {
        class: "McpSigV4Error".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| failure(&Failure::Runtime(error)))?;
        let recorder = Recorder::default();
        let outcome = runtime.block_on(sign_requests(input, &recorder));
        let sts_calls = recorder.calls();
        match outcome {
            Ok(signed) => Ok(json!({"error": null, "sts_calls": sts_calls, "signed": signed})),
            Err(Failure::Sign(McpSigV4Error::NoCredentials)) => {
                Ok(json!({"error": "ValueError", "sts_calls": sts_calls, "signed": []}))
            }
            Err(error) => Err(failure(&error)),
        }
    }
}
