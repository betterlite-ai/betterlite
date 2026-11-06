#![cfg(test)]

use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aws_smithy_async::rt::sleep::{SharedAsyncSleep, TokioSleep};
use aws_smithy_async::time::{SharedTimeSource, TimeSource};
use aws_smithy_runtime_api::client::http::{
    HttpConnector, HttpConnectorFuture, SharedHttpConnector, http_client_fn,
};
use aws_smithy_runtime_api::client::orchestrator::{HttpRequest, HttpResponse};
use aws_smithy_runtime_api::client::result::ConnectorError;
use aws_smithy_runtime_api::http::StatusCode;
use aws_smithy_types::body::SdkBody;
use aws_types::os_shim_internal::Fs;
use bytes::Bytes;
use http::Request;
use secrecy::SecretString;

use super::super::{AwsSources, McpSigV4Auth, McpSigV4Settings};

// 2015-08-30T12:36:00Z, the date of the AWS SigV4 test suite
pub(super) const VECTOR_TIME: u64 = 1_440_938_160;

#[derive(Debug, Clone)]
pub(super) struct Clock(Arc<AtomicU64>);

impl Clock {
    pub(super) fn at(seconds: u64) -> Self {
        Self(Arc::new(AtomicU64::new(seconds)))
    }

    pub(super) fn set(&self, seconds: u64) {
        self.0.store(seconds, Ordering::SeqCst);
    }
}

impl TimeSource for Clock {
    fn now(&self) -> SystemTime {
        UNIX_EPOCH
            .checked_add(Duration::from_secs(self.0.load(Ordering::SeqCst)))
            .unwrap()
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Reply {
    Sts,
    Status(u16, &'static str),
    Refused,
}

#[derive(Clone, Debug)]
pub(super) struct Call {
    pub(super) uri: String,
    pub(super) authorization: String,
    pub(super) security_token: String,
    pub(super) body: String,
}

#[derive(Clone, Debug, Default)]
pub(super) struct Recorder(Arc<Mutex<Vec<Call>>>);

impl Recorder {
    pub(super) fn calls(&self) -> Vec<Call> {
        self.0.lock().unwrap().clone()
    }
}

#[derive(Debug)]
struct Canned {
    reply: Reply,
    recorder: Recorder,
}

impl HttpConnector for Canned {
    fn call(&self, request: HttpRequest) -> HttpConnectorFuture {
        let text = |name: &str| request.headers().get(name).unwrap_or_default().to_owned();
        self.recorder.0.lock().unwrap().push(Call {
            uri: request.uri().to_owned(),
            authorization: text("authorization"),
            security_token: text("x-amz-security-token"),
            body: String::from_utf8(request.body().bytes().unwrap_or_default().to_vec()).unwrap(),
        });
        let (status, xml) = match self.reply {
            Reply::Sts => (
                200,
                "<AssumeRoleResponse><AssumeRoleResult><Credentials><AccessKeyId>ASIATEMP</AccessKeyId><SecretAccessKey>tempsecret</SecretAccessKey><SessionToken>temptoken</SessionToken><Expiration>2030-01-01T00:00:00Z</Expiration></Credentials></AssumeRoleResult></AssumeRoleResponse>",
            ),
            Reply::Status(status, xml) => (status, xml),
            Reply::Refused => {
                return HttpConnectorFuture::ready(Err(ConnectorError::other(
                    Box::new(io::Error::other("fixture refused")),
                    None,
                )));
            }
        };
        let response = HttpResponse::new(StatusCode::try_from(status).unwrap(), SdkBody::from(xml));
        HttpConnectorFuture::ready(Ok(response))
    }
}

pub(super) fn sources(
    environment: &[(&str, &str)],
    files: &[(&str, &str)],
    clock: &Clock,
    reply: Reply,
) -> (AwsSources, Recorder) {
    let recorder = Recorder::default();
    let shared = recorder.clone();
    let sources = AwsSources {
        environment: environment
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect::<HashMap<_, _>>(),
        filesystem: Fs::from_slice(files),
        client: http_client_fn(move |_, _| {
            SharedHttpConnector::new(Canned {
                reply,
                recorder: shared.clone(),
            })
        }),
        clock: SharedTimeSource::new(clock.clone()),
        sleep: SharedAsyncSleep::new(TokioSleep::new()),
        timeout: Duration::from_secs(5),
    };
    (sources, recorder)
}

pub(super) fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

pub(super) const fn blank_settings() -> McpSigV4Settings {
    McpSigV4Settings {
        aws_access_key_id: None,
        aws_secret_access_key: None,
        aws_session_token: None,
        aws_region_name: None,
        aws_service_name: None,
        aws_role_name: None,
        aws_session_name: None,
    }
}

pub(super) fn explicit_settings() -> McpSigV4Settings {
    McpSigV4Settings {
        aws_access_key_id: Some(secret("AKIDEXAMPLE")),
        aws_secret_access_key: Some(secret("wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY")),
        ..blank_settings()
    }
}

pub(super) async fn built(
    settings: McpSigV4Settings,
    environment: &[(&str, &str)],
    reply: Reply,
) -> (McpSigV4Auth, Recorder, Clock) {
    let clock = Clock::at(VECTOR_TIME);
    let (sources, recorder) = sources(environment, &[], &clock, reply);
    let auth = McpSigV4Auth::new(settings, sources).await.unwrap();
    (auth, recorder, clock)
}

pub(super) fn mcp_request(body: &'static str) -> Request<Bytes> {
    Request::post("https://mcp.example.com/mcp")
        .header("content-type", "application/json")
        .body(Bytes::from_static(body.as_bytes()))
        .unwrap()
}
