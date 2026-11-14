use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, UNIX_EPOCH};

use aws_smithy_async::time::{SharedTimeSource, StaticTimeSource};
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use betterlite_openai::transcription::transport::TranscriptionTransport;
use http::{HeaderMap, Method};
use secrecy::SecretString;

use super::support::{env, params};
use crate::aws_settings::sources::Credentials;
use crate::credentials::error::CredentialError;
use crate::transcription::audio::{AudioFormat, AudioPayload};
use crate::transcription::auth::ChainCredentials;
use crate::transcription::binding::{
    BindingArguments, TranscriptionBinding, TranscriptionCall, async_audio_transcriptions,
};
use crate::transcription::call::NativeTranscription;
use crate::transcription::error::{CallError, TranscriptionError};
use crate::transcription::params::Params;

const HOSTED: &str = r#"{"aws_access_key_id":"AKIDONE","aws_secret_access_key":"secret-one-value","aws_session_token":"token-one-value"}"#;
const CHAINED: &str = r#"{"aws_role_name":"role-a","aws_region_name":"eu-west-1"}"#;
const REPLY: &str = r#"{"output":{"message":{"content":[{"text":"hello world"}]}},"usage":{"inputTokens":1,"outputTokens":2}}"#;
const SCOPE: &str = "20150819/us-east-1/bedrock/aws4_request";

struct Script {
    status: u16,
    body: Vec<u8>,
    seen: Mutex<Vec<OutboundRequest>>,
}

impl Script {
    fn replying(status: u16, body: &[u8]) -> Self {
        Self {
            status,
            body: body.to_vec(),
            seen: Mutex::new(Vec::new()),
        }
    }

    fn sent(&self) -> Vec<OutboundRequest> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain(..)
            .collect()
    }
}

impl TranscriptionTransport for Script {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        ready(Ok(UpstreamResponse {
            status: self.status,
            headers: HeaderMap::new(),
            body: self.body.clone(),
        }))
    }
}

struct FakeChain {
    asked: Mutex<Vec<AwsSettings>>,
}

impl ChainCredentials for FakeChain {
    fn resolve(
        &self,
        settings: &AwsSettings,
    ) -> impl Future<Output = Result<Credentials, CredentialError>> + Send {
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(settings.clone());
        ready(Ok(Credentials {
            access_key_id: SecretString::from("AKIDCHAIN".to_owned()),
            secret_access_key: SecretString::from("chain-secret".to_owned()),
            session_token: None,
        }))
    }
}

fn native(status: u16, body: &[u8]) -> NativeTranscription<Script, FakeChain> {
    NativeTranscription {
        transport: Script::replying(status, body),
        credentials: FakeChain {
            asked: Mutex::new(Vec::new()),
        },
        environment: env(&[("AWS_REGION_NAME", "us-east-1")]),
        clock: SharedTimeSource::new(StaticTimeSource::new(
            UNIX_EPOCH
                .checked_add(Duration::from_hours(400_000))
                .unwrap(),
        )),
    }
}

fn call<'call>(options: &'call Params, headers: Option<&'call Params>) -> TranscriptionCall<'call> {
    TranscriptionCall {
        model: "bedrock/m",
        api_key: None,
        api_base: None,
        custom_llm_provider: "bedrock",
        extra_headers: headers,
        optional_params: options,
        timeout: None,
    }
}

fn arguments<'call>(
    call: &'call TranscriptionCall<'call>,
    timeout_seconds: Option<f64>,
) -> BindingArguments<'call> {
    BindingArguments {
        audio: AudioPayload {
            data: "QQ==".to_owned(),
            format: AudioFormat::Wav,
            filename: "clip.wav".to_owned(),
        },
        call,
        timeout_seconds,
    }
}

fn header(request: &OutboundRequest, name: &str) -> Option<String> {
    request
        .headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

#[tokio::test]
async fn host_supplied_call_signs_and_decodes_the_reply() {
    let binding = native(200, REPLY.as_bytes());
    let options = params(HOSTED);
    let call = call(&options, None);
    let reply = async_audio_transcriptions(&binding, &arguments(&call, None))
        .await
        .unwrap();
    assert_eq!(reply.text.as_deref(), Some("hello world"));
    let sent = binding.transport.sent();
    assert_eq!(sent.len(), 1);
    let request = &sent[0];
    assert_eq!(request.method, Method::POST);
    assert_eq!(
        request.url.as_str(),
        "https://bedrock-runtime.us-east-1.amazonaws.com/model/m/converse"
    );
    assert_eq!(request.deadline, Duration::from_mins(10));
    let authorization = header(request, "authorization").unwrap();
    assert!(
        authorization.contains(&format!("Credential=AKIDONE/{SCOPE}")),
        "{authorization}"
    );
    assert_eq!(
        header(request, "x-amz-security-token").as_deref(),
        Some("token-one-value")
    );
    assert_eq!(
        header(request, "content-type").as_deref(),
        Some("application/json")
    );
    let RequestBody::Json(body) = &request.body else {
        panic!("a JSON body is expected");
    };
    let sent_body: Json = serde_json::from_slice(body).unwrap();
    let Json::Object(fields) = sent_body else {
        panic!("an object body is expected");
    };
    assert!(fields.contains_key("messages") && fields.contains_key("inferenceConfig"));
    assert!(
        String::from_utf8(body.clone())
            .unwrap()
            .contains(r#""bytes":"QQ==""#)
    );
    assert!(binding.credentials.asked.lock().unwrap().is_empty());
}

#[tokio::test]
async fn chain_call_resolves_credentials_with_the_call_settings() {
    let binding = native(200, REPLY.as_bytes());
    let options = params(CHAINED);
    let call = call(&options, None);
    let reply = binding.atranscribe(&arguments(&call, None)).await.unwrap();
    assert_eq!(
        reply.get("text"),
        Some(&Json::Text("hello world".to_owned()))
    );
    let roles: Vec<Option<String>> = binding
        .credentials
        .asked
        .lock()
        .unwrap()
        .iter()
        .map(|settings| settings.role_name.clone())
        .collect();
    assert_eq!(roles, [Some("role-a".to_owned())]);
    let sent = binding.transport.sent();
    let authorization = header(&sent[0], "authorization").unwrap();
    assert!(
        authorization.contains("Credential=AKIDCHAIN/20150819/eu-west-1/bedrock/aws4_request"),
        "{authorization}"
    );
    assert_eq!(header(&sent[0], "x-amz-security-token"), None);
}

#[tokio::test]
async fn timeout_seconds_set_the_request_deadline() {
    let binding = native(200, REPLY.as_bytes());
    let options = params(HOSTED);
    let call = call(&options, None);
    binding
        .atranscribe(&arguments(&call, Some(2.5)))
        .await
        .unwrap();
    assert_eq!(
        binding.transport.sent()[0].deadline,
        Duration::from_millis(2500)
    );
    let failure = binding
        .atranscribe(&arguments(&call, Some(-1.0)))
        .await
        .unwrap_err();
    assert!(matches!(failure, CallError::Timeout { .. }), "{failure}");
    assert!(binding.transport.sent().is_empty());
}

#[tokio::test]
async fn extra_headers_must_be_strings_and_are_sent_signed() {
    let binding = native(200, REPLY.as_bytes());
    let options = params(HOSTED);
    let headers = params(r#"{"x-trace":"abc"}"#);
    let call_with = call(&options, Some(&headers));
    binding
        .atranscribe(&arguments(&call_with, None))
        .await
        .unwrap();
    let sent = binding.transport.sent();
    assert_eq!(header(&sent[0], "x-trace").as_deref(), Some("abc"));
    let authorization = header(&sent[0], "authorization").unwrap();
    assert!(authorization.contains("x-trace"), "{authorization}");
    let bad = params(r#"{"x-count":1}"#);
    let call_bad = call(&options, Some(&bad));
    let failure = binding
        .atranscribe(&arguments(&call_bad, None))
        .await
        .unwrap_err();
    assert!(
        matches!(&failure, CallError::HeaderType { name, actual: "number" } if name == "x-count"),
        "{failure}"
    );
    assert!(binding.transport.sent().is_empty());
}

#[tokio::test]
async fn non_success_status_reports_the_truncated_body() {
    let long = "e".repeat(300);
    let binding = native(403, long.as_bytes());
    let options = params(HOSTED);
    let call = call(&options, None);
    let failure = binding
        .atranscribe(&arguments(&call, None))
        .await
        .unwrap_err();
    let CallError::Status { status, body } = failure else {
        panic!("a status error is expected");
    };
    assert_eq!(status, 403);
    assert_eq!(body, format!("{}... (truncated)", "e".repeat(256)));
    let exact = "e".repeat(256);
    let binding = native(500, exact.as_bytes());
    let failure = binding
        .atranscribe(&arguments(&call, None))
        .await
        .unwrap_err();
    assert!(matches!(&failure, CallError::Status { status: 500, body } if *body == exact));
}

#[tokio::test]
async fn the_status_boundary_sits_at_300() {
    let options = params(HOSTED);
    let call = call(&options, None);
    let accepted = native(299, REPLY.as_bytes());
    assert!(accepted.atranscribe(&arguments(&call, None)).await.is_ok());
    let rejected = native(300, REPLY.as_bytes());
    assert!(matches!(
        rejected.atranscribe(&arguments(&call, None)).await,
        Err(CallError::Status { status: 300, .. })
    ));
}

#[tokio::test]
async fn an_undecodable_reply_is_a_transcription_error() {
    let binding = native(200, b"{}");
    let options = params(HOSTED);
    let call = call(&options, None);
    let failure = binding
        .atranscribe(&arguments(&call, None))
        .await
        .unwrap_err();
    assert!(matches!(
        failure,
        CallError::Transcription(TranscriptionError::InvalidResponse(_))
    ));
}

#[test]
fn the_sync_call_refuses_to_block_on_the_async_transport() {
    let binding = native(200, REPLY.as_bytes());
    let options = params(HOSTED);
    let call = call(&options, None);
    assert!(matches!(
        binding.transcribe(&arguments(&call, None)),
        Err(CallError::Blocking)
    ));
}
