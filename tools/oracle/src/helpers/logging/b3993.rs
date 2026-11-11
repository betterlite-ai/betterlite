use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use betterlite_logging::s3_transport::batch_element::{ContentType, S3BatchLoggingElement};
use betterlite_logging::s3_transport::encryption::{
    ServerSideEncryption, resolve_server_side_encryption,
};
use betterlite_logging::s3_transport::error::S3TransportError;
use betterlite_logging::s3_transport::option_value::OptionValue;
use betterlite_logging::s3_transport::signing::{
    Clock, CredentialSource, Credentials, StaticCredentials,
};
use betterlite_logging::s3_transport::transport::{
    S3HttpClient, S3Request, S3Response, S3Transport, ThrottleSink, UploadOutcome,
};
use betterlite_logging::s3_transport::url_construction::S3Endpoint;
use betterlite_models::decisions::Json;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    operation: Operation,
    settings: Settings,
    credentials: Keys,
    now: u64,
    #[serde(default)]
    element: Option<Element>,
    #[serde(default)]
    object_key: Option<String>,
    responses: Vec<Reply>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    Put,
    Get,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    bucket_name: String,
    region_name: String,
    #[serde(default)]
    endpoint_url: Option<String>,
    #[serde(default)]
    use_virtual_hosted_style: bool,
    #[serde(default)]
    server_side_encryption: Option<String>,
    #[serde(default)]
    sse_kms_key_id: Option<String>,
    #[serde(default = "default_drop_on_terminal_error")]
    drop_on_terminal_error: bool,
    #[serde(default)]
    adaptive_concurrency: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Keys {
    access_key_id: String,
    secret_access_key: String,
    #[serde(default)]
    session_token: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Element {
    s3_object_key: String,
    s3_object_download_filename: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    content_type: ContentType,
    #[serde(default)]
    payload: Option<Json>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    status: u16,
    #[serde(default)]
    body: String,
}

#[derive(Debug, Default)]
struct ScriptedS3 {
    replies: Mutex<VecDeque<Reply>>,
    sent: Mutex<Vec<S3Request>>,
}

impl ScriptedS3 {
    fn new(replies: Vec<Reply>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies)),
            sent: Mutex::new(Vec::new()),
        }
    }

    fn sent(&self) -> Vec<S3Request> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl S3HttpClient for ScriptedS3 {
    fn send(
        &self,
        request: S3Request,
    ) -> Pin<Box<dyn Future<Output = Result<S3Response, S3TransportError>> + Send + '_>> {
        Box::pin(async move {
            self.sent
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(request);
            let reply = self
                .replies
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pop_front()
                .unwrap_or(Reply {
                    status: 200,
                    body: String::new(),
                });
            Ok(S3Response {
                status: reply.status,
                body: reply.body.into_bytes(),
            })
        })
    }
}

#[derive(Debug, Default)]
struct RecordingSink {
    samples: Mutex<Vec<bool>>,
}

impl RecordingSink {
    fn samples(&self) -> Vec<bool> {
        self.samples
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ThrottleSink for RecordingSink {
    fn record(&self, throttled: bool) {
        self.samples
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(throttled);
    }
}

#[derive(Debug)]
struct FixedClock(SystemTime);

impl Clock for FixedClock {
    fn now(&self) -> SystemTime {
        self.0
    }
}

#[derive(Serialize)]
struct Recorded {
    method: String,
    url: String,
    body: Option<String>,
    headers: BTreeMap<String, String>,
}

const fn default_drop_on_terminal_error() -> bool {
    true
}

fn failure(class: &str, message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .start_paused(true)
            .build()
            .map_err(|error| failure("RuntimeError", &error.to_string()))?;
        runtime.block_on(execute(input))
    }
}

async fn execute(input: &Input) -> HelperResult {
    let sink = Arc::new(RecordingSink::default());
    let scripted = Arc::new(ScriptedS3::new(input.responses.clone()));
    let client: Arc<dyn S3HttpClient> = scripted.clone();
    let throttle: Option<Arc<dyn ThrottleSink>> = if input.settings.adaptive_concurrency {
        let recording: Arc<dyn ThrottleSink> = sink.clone();
        Some(recording)
    } else {
        None
    };
    let credentials: Arc<dyn CredentialSource> = Arc::new(StaticCredentials {
        credentials: Credentials {
            access_key_id: SecretString::from(input.credentials.access_key_id.clone()),
            secret_access_key: SecretString::from(input.credentials.secret_access_key.clone()),
            session_token: input
                .credentials
                .session_token
                .as_ref()
                .map(|token| SecretString::from(token.clone())),
        },
    });
    let transport = S3Transport {
        client,
        endpoint: S3Endpoint {
            bucket: input.settings.bucket_name.clone(),
            region: input.settings.region_name.clone(),
            endpoint_url: input.settings.endpoint_url.clone(),
            virtual_hosted_style: input.settings.use_virtual_hosted_style,
        },
        credentials,
        encryption: encryption(input),
        drop_on_terminal_error: input.settings.drop_on_terminal_error,
        throttle,
        clock: Arc::new(FixedClock(
            SystemTime::UNIX_EPOCH
                .checked_add(Duration::from_secs(input.now))
                .ok_or_else(|| failure("ValueError", "now is too large"))?,
        )),
    };
    let mut outcome = None;
    let mut samples = None;
    let mut object = None;
    match input.operation {
        Operation::Put => {
            let element = input
                .element
                .as_ref()
                .ok_or_else(|| failure("ValueError", "a put call needs an element"))?;
            let logged = S3BatchLoggingElement {
                payload: element.payload.clone().unwrap_or(Json::Null),
                s3_object_key: element.s3_object_key.clone(),
                s3_object_download_filename: element.s3_object_download_filename.clone(),
                body: element.body.clone(),
                content_type: element.content_type,
                retrying_since: None,
            };
            outcome = Some(match transport.async_upload_data_to_s3(&logged).await {
                UploadOutcome::Delivered => "delivered",
                UploadOutcome::Retry => "retry",
                UploadOutcome::Dropped => "dropped",
            });
            if input.settings.adaptive_concurrency {
                samples = Some(sink.samples());
            }
        }
        Operation::Get => {
            let object_key = input
                .object_key
                .as_ref()
                .ok_or_else(|| failure("ValueError", "a get call needs an object_key"))?;
            object = transport.download_object(object_key).await;
        }
    }
    let requests: Vec<Recorded> = scripted
        .sent()
        .into_iter()
        .map(|request| Recorded {
            method: request.method.as_str().to_owned(),
            url: request.url,
            body: match input.operation {
                Operation::Put => Some(String::from_utf8_lossy(&request.body).into_owned()),
                Operation::Get => None,
            },
            headers: request
                .headers
                .into_iter()
                .map(|(name, value)| (name.to_ascii_lowercase(), value))
                .collect(),
        })
        .collect();
    Ok(json!({
        "requests": requests,
        "outcome": outcome,
        "samples": samples,
        "object": object,
    }))
}

fn encryption(input: &Input) -> Option<ServerSideEncryption> {
    let algorithm = input
        .settings
        .server_side_encryption
        .as_ref()
        .map(|text| OptionValue::Text(text.clone()));
    let kms_key = input
        .settings
        .sse_kms_key_id
        .as_ref()
        .map(|text| OptionValue::Text(text.clone()));
    resolve_server_side_encryption(algorithm.as_ref(), kms_key.as_ref())
}
