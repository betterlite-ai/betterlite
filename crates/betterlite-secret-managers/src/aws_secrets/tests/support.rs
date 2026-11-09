use std::collections::{BTreeMap, VecDeque};
use std::convert::Infallible;
use std::future::ready;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aws_credential_types::Credentials;
use http::StatusCode;
use secrecy::SecretString;
use serde::Deserialize;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::{MakeWriter, fmt};
use uuid::Uuid;

use crate::aws::request_signing::{
    AwsCredentialsInfo, AwsOptionalParams, AwsTransport, CredentialResolver, TransportError,
    TransportFuture, TransportRequest, TransportResponse,
};
use crate::aws::{AwsDependencies, AwsSecretsManager};
use crate::aws_secret_reads::blocking::AwsBlockingTransport;
use crate::aws_secrets::AwsSecretClient;
use crate::secret_contracts::options::{SecretOptions, SecretTags, SecretTimeout, SecretWrite};

const ACCESS_KEY: &str = "AKIDEXAMPLE";
const SECRET_KEY: &str = "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY";

pub(super) const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);
pub(super) const DEFAULT_URL: &str = "https://secretsmanager.us-east-1.amazonaws.com";
pub(super) const FIRST_TOKEN: &str = "00000000-0000-4000-8000-000000000001";
pub(super) const SECOND_TOKEN: &str = "00000000-0000-4000-8000-000000000002";

pub(super) type Reply = Result<TransportResponse, TransportError<io::Error>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Sent {
    pub(super) url: String,
    pub(super) target: Option<String>,
    pub(super) body: String,
    pub(super) timeout: Duration,
}

#[derive(Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "PascalCase", deny_unknown_fields)]
pub(super) struct SentBody {
    pub(super) name: Option<String>,
    pub(super) secret_id: Option<String>,
    pub(super) secret_string: Option<String>,
    pub(super) client_request_token: Option<String>,
    pub(super) description: Option<String>,
    pub(super) kms_key_id: Option<String>,
    pub(super) tags: Option<Vec<SentTag>>,
    pub(super) add_replica_regions: Option<Vec<SentRegion>>,
    pub(super) recovery_window_in_days: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(super) struct SentTag {
    pub(super) key: String,
    pub(super) value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(super) struct SentRegion {
    pub(super) region: String,
}

#[derive(Debug, Default)]
pub(super) struct Settings {
    pub(super) kms_key_id: Option<String>,
    pub(super) replica_regions: Vec<String>,
}

pub(super) struct Resolver;

impl CredentialResolver for Resolver {
    type Error = Infallible;

    fn resolve_credentials(
        &self,
        optional_params: &mut AwsOptionalParams,
    ) -> Result<AwsCredentialsInfo, Infallible> {
        Ok(AwsCredentialsInfo {
            aws_region_name: optional_params
                .aws_region_name
                .clone()
                .unwrap_or_else(|| "us-east-1".to_owned()),
            aws_bedrock_runtime_endpoint: None,
            credentials: Credentials::new(ACCESS_KEY, SECRET_KEY, None, None, "test"),
        })
    }
}

pub(super) struct FakeTransport {
    replies: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<Sent>>,
}

impl FakeTransport {
    pub(super) fn new(replies: Vec<Reply>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(VecDeque::from(replies)),
            requests: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn sent(&self) -> Vec<Sent> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn next(&self) -> Reply {
        self.replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_else(|| answer(500, "no reply left"))
    }

    fn record(&self, request: &TransportRequest<'_>) {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Sent {
                url: request.url.to_owned(),
                target: request
                    .headers
                    .get("x-amz-target")
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_owned),
                body: String::from_utf8(request.body.to_vec()).unwrap_or_default(),
                timeout: request.timeout,
            });
    }
}

impl AwsTransport for FakeTransport {
    type Error = io::Error;

    fn post<'request>(
        &'request self,
        request: TransportRequest<'request>,
    ) -> TransportFuture<'request, io::Error> {
        self.record(&request);
        Box::pin(ready(self.next()))
    }
}

impl AwsBlockingTransport for FakeTransport {
    fn post_blocking(&self, request: TransportRequest<'_>) -> Reply {
        self.record(&request);
        self.next()
    }
}

pub(super) fn answer(status: u16, body: &str) -> Reply {
    let code = StatusCode::from_u16(status)
        .map_err(|error| TransportError::Failed(io::Error::other(error)))?;
    Ok(TransportResponse {
        status: code,
        body: body.as_bytes().to_vec(),
    })
}

pub(super) fn timed_out() -> Reply {
    Err(TransportError::Timeout)
}

pub(super) fn region_params(region: &str) -> AwsOptionalParams {
    AwsOptionalParams {
        aws_role_name: None,
        aws_session_name: None,
        aws_region_name: Some(region.to_owned()),
        aws_external_id: None,
        aws_profile_name: None,
        aws_web_identity_token: None,
        aws_sts_endpoint: None,
        aws_access_key_id: None,
        aws_secret_access_key: None,
        aws_session_token: None,
        aws_bedrock_runtime_endpoint: None,
    }
}

pub(super) fn first_token() -> Uuid {
    Uuid::from_u128(0x0000_0000_0000_4000_8000_0000_0000_0001)
}

pub(super) fn second_token() -> Uuid {
    Uuid::from_u128(0x0000_0000_0000_4000_8000_0000_0000_0002)
}

pub(super) fn secret_client(
    manager: &AwsSecretsManager<Resolver, FakeTransport>,
) -> AwsSecretClient<'_, Resolver, FakeTransport> {
    AwsSecretClient {
        manager,
        next_token: first_token,
    }
}

pub(super) fn manager(
    transport: &Arc<FakeTransport>,
    settings: Settings,
) -> AwsSecretsManager<Resolver, FakeTransport> {
    AwsSecretsManager {
        aws_region_name: None,
        aws_role_name: None,
        aws_session_name: None,
        aws_external_id: None,
        aws_profile_name: None,
        aws_web_identity_token: None,
        aws_sts_endpoint: None,
        replica_regions: settings.replica_regions,
        kms_key_id: settings.kms_key_id,
        dependencies: AwsDependencies {
            environment: BTreeMap::new(),
            transport: Arc::clone(transport),
            resolver: Resolver,
            dns_suffix,
            clock: signing_time,
        },
    }
}

fn dns_suffix(_region: &str) -> &'static str {
    "amazonaws.com"
}

fn signing_time() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(1_709_641_845))
        .unwrap_or(UNIX_EPOCH)
}

pub(super) fn sent_body(text: &str) -> SentBody {
    serde_json::from_str(text).expect("the request body matches the modelled wire shape")
}

pub(super) fn bodies(sent: &[Sent]) -> Vec<SentBody> {
    sent.iter().map(|record| sent_body(&record.body)).collect()
}

pub(super) fn actions(sent: &[Sent]) -> Vec<String> {
    sent.iter()
        .map(|record| record.target.clone().unwrap_or_default())
        .collect()
}

pub(super) fn write_request<'borrowed>(
    name: &'borrowed str,
    value: &'borrowed SecretString,
    tags: Option<&'borrowed SecretTags>,
    timeout: Option<SecretTimeout>,
) -> SecretWrite<'borrowed, AwsOptionalParams> {
    SecretWrite {
        secret_name: name,
        secret_value: value,
        description: None,
        options: SecretOptions {
            optional_params: None,
            timeout,
        },
        tags,
    }
}

#[derive(Clone, Default)]
pub(super) struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    pub(super) fn text(&self) -> String {
        String::from_utf8(
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        )
        .unwrap_or_default()
    }
}

impl io::Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for LogBuffer {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

pub(super) fn capture_logs() -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}
