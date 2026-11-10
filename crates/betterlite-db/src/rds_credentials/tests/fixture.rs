#![cfg(test)]

use std::collections::BTreeMap;
use std::io::{Result as IoResult, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aws_smithy_async::rt::sleep::{SharedAsyncSleep, TokioSleep};
use aws_smithy_async::time::{SharedTimeSource, TimeSource};
use aws_smithy_runtime_api::client::http::{
    HttpConnector, HttpConnectorFuture, SharedHttpConnector, http_client_fn,
};
use aws_smithy_runtime_api::client::orchestrator::{HttpRequest, HttpResponse};
use aws_smithy_runtime_api::http::StatusCode;
use aws_smithy_types::body::SdkBody;
use aws_types::os_shim_internal::Fs;
use betterlite_secret_managers::secret_lookup::SecretValue;
use betterlite_secret_managers::secret_lookup::error::SecretLookupError;
use secrecy::SecretString;
use tracing_subscriber::fmt::MakeWriter;

use super::super::secrets::RdsSecrets;
use super::super::{RdsCredentialInputs, RdsEndpoint, RdsSources, RdsTimeout};

pub(super) const WEB_IDENTITY_XML: &str = "<AssumeRoleWithWebIdentityResponse><AssumeRoleWithWebIdentityResult><Credentials><AccessKeyId>ASIAWEB</AccessKeyId><SecretAccessKey>websecret</SecretAccessKey><SessionToken>webtoken</SessionToken><Expiration>2030-01-01T00:00:00Z</Expiration></Credentials></AssumeRoleWithWebIdentityResult></AssumeRoleWithWebIdentityResponse>";

pub(super) const ASSUME_ROLE_XML: &str = "<AssumeRoleResponse><AssumeRoleResult><Credentials><AccessKeyId>ASIAROLE</AccessKeyId><SecretAccessKey>rolesecret</SecretAccessKey><SessionToken>roletoken</SessionToken><Expiration>2030-01-01T00:00:00Z</Expiration></Credentials></AssumeRoleResult></AssumeRoleResponse>";

const MISSING_SESSION_TOKEN_XML: &str = "<AssumeRoleResponse><AssumeRoleResult><Credentials><AccessKeyId>ASIAROLE</AccessKeyId><SecretAccessKey>rolesecret</SecretAccessKey></Credentials></AssumeRoleResult></AssumeRoleResponse>";

const MISSING_ACCESS_KEY_XML: &str = "<AssumeRoleResponse><AssumeRoleResult><Credentials><SecretAccessKey>rolesecret</SecretAccessKey><SessionToken>roletoken</SessionToken></Credentials></AssumeRoleResult></AssumeRoleResponse>";

const WITHOUT_CREDENTIALS_XML: &str =
    "<AssumeRoleResponse><AssumeRoleResult></AssumeRoleResult></AssumeRoleResponse>";

pub(super) const ACCESS_KEY: &str = "AKIDEXAMPLE";

// the STS client signs with the ambient chain, so every test that reaches STS needs these
pub(super) const SIGNER_CREDENTIALS: [(&str, &str); 3] = [
    ("AWS_ACCESS_KEY_ID", "AKIDSIGNER"),
    ("AWS_SECRET_ACCESS_KEY", "signersecret"),
    ("AWS_EC2_METADATA_DISABLED", "true"),
];
pub(super) const SECRET_KEY: &str = "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY";

pub(super) fn botocore_fixture() -> BotocoreFixture {
    serde_json::from_str(include_str!("../fixtures/botocore-1.43.118.json"))
        .expect("the pinned botocore fixture parses")
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct BotocoreFixture {
    pub(super) provenance: Provenance,
    pub(super) fixed_time_unix_seconds: u64,
    pub(super) tokens: Vec<TokenVector>,
    pub(super) sts: StsFixture,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct Provenance {
    pub(super) python: String,
    pub(super) boto3: String,
    pub(super) botocore: String,
    pub(super) fixed_time: String,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct TokenVector {
    pub(super) name: String,
    pub(super) source: String,
    pub(super) access_key_id: String,
    pub(super) secret_access_key: String,
    pub(super) session_token: Option<String>,
    pub(super) region: String,
    pub(super) host: String,
    pub(super) port: String,
    pub(super) user: String,
    pub(super) token: String,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct StsFixture {
    pub(super) region: String,
    pub(super) assume_role_with_web_identity: WebIdentityRequest,
    pub(super) assume_role: AssumeRoleRequest,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct WebIdentityRequest {
    pub(super) role_arn: String,
    pub(super) role_session_name: String,
    pub(super) web_identity_token: String,
    pub(super) duration_seconds: i32,
    pub(super) url: String,
    pub(super) body: String,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct AssumeRoleRequest {
    pub(super) role_arn: String,
    pub(super) role_session_name: String,
    pub(super) url: String,
    pub(super) body: String,
}

#[derive(Debug, Clone)]
pub(super) struct Clock(Arc<AtomicU64>);

impl Clock {
    pub(super) fn at(seconds: u64) -> Self {
        Self(Arc::new(AtomicU64::new(seconds)))
    }
}

impl TimeSource for Clock {
    fn now(&self) -> SystemTime {
        UNIX_EPOCH
            .checked_add(Duration::from_secs(self.0.load(Ordering::SeqCst)))
            .expect("the fixture clock fits the system time range")
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) enum Reply {
    WebIdentity,
    AssumeRole,
    Status(u16, &'static str),
    MissingSessionToken,
    MissingAccessKey,
    WithoutCredentials,
}

impl Reply {
    fn response(self) -> (u16, &'static str) {
        match self {
            Self::WebIdentity => (200, WEB_IDENTITY_XML),
            Self::AssumeRole => (200, ASSUME_ROLE_XML),
            Self::Status(status, body) => (status, body),
            Self::MissingSessionToken => (200, MISSING_SESSION_TOKEN_XML),
            Self::MissingAccessKey => (200, MISSING_ACCESS_KEY_XML),
            Self::WithoutCredentials => (200, WITHOUT_CREDENTIALS_XML),
        }
    }
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
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Debug)]
struct Canned {
    reply: Reply,
    recorder: Recorder,
}

impl HttpConnector for Canned {
    fn call(&self, request: HttpRequest) -> HttpConnectorFuture {
        let header = |name: &str| request.headers().get(name).unwrap_or_default().to_owned();
        let body = request.body().bytes().unwrap_or_default().to_vec();
        self.recorder
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Call {
                uri: request.uri().to_owned(),
                authorization: header("authorization"),
                security_token: header("x-amz-security-token"),
                body: String::from_utf8(body).expect("the SDK body is form-encoded text"),
            });
        let (status, xml) = self.reply.response();
        let response = HttpResponse::new(
            StatusCode::try_from(status).expect("a fixture status is a valid HTTP status"),
            SdkBody::from(xml),
        );
        HttpConnectorFuture::ready(Ok(response))
    }
}

pub(super) struct RecordingSecrets {
    answers: Mutex<BTreeMap<String, Option<SecretValue>>>,
    calls: Mutex<Vec<String>>,
    failing: bool,
}

impl RecordingSecrets {
    pub(super) fn new(answers: &[(&str, Option<&str>)]) -> Self {
        let mut map = BTreeMap::new();
        for (name, value) in answers {
            map.insert(
                (*name).to_owned(),
                value.map(|text| SecretValue::Text(secret(text))),
            );
        }
        Self {
            answers: Mutex::new(map),
            calls: Mutex::new(Vec::new()),
            failing: false,
        }
    }

    pub(super) fn failing() -> Self {
        Self {
            answers: Mutex::new(BTreeMap::new()),
            calls: Mutex::new(Vec::new()),
            failing: true,
        }
    }

    pub(super) fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl RdsSecrets for RecordingSecrets {
    fn get_secret(&self, name: &str) -> Result<Option<SecretValue>, SecretLookupError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(name.to_owned());
        if self.failing {
            return Err(SecretLookupError::UnsupportedOidcProvider);
        }
        Ok(self
            .answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(name)
            .cloned()
            .unwrap_or(None))
    }
}

pub(super) fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

#[derive(Clone, Default)]
pub(super) struct LogCapture(Arc<Mutex<Vec<u8>>>);

impl LogCapture {
    pub(super) fn contents(&self) -> String {
        let bytes = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        String::from_utf8(bytes).expect("the log writer records UTF-8")
    }
}

impl Write for LogCapture {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for LogCapture {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

pub(super) fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn aws_sources<'source>(
    secrets: &'source dyn RdsSecrets,
    environment: &'source BTreeMap<String, String>,
    files: &[(&str, &str)],
    clock: &Clock,
    reply: Reply,
) -> (RdsSources<'source>, Recorder) {
    let recorder = Recorder::default();
    let shared = recorder.clone();
    let sources = RdsSources {
        secrets,
        environment,
        filesystem: Fs::from_slice(files),
        client: http_client_fn(move |_, _| {
            SharedHttpConnector::new(Canned {
                reply,
                recorder: shared.clone(),
            })
        }),
        clock: SharedTimeSource::new(clock.clone()),
        sleep: SharedAsyncSleep::new(TokioSleep::new()),
    };
    (sources, recorder)
}

#[derive(Clone, Copy)]
pub(super) enum Preset {
    Direct,
    WebIdentity,
    AssumeRole,
}

pub(super) fn preset_inputs(preset: Preset) -> RdsCredentialInputs {
    match preset {
        Preset::Direct => RdsCredentialInputs {
            access_key_id: Some(secret(ACCESS_KEY)),
            secret_access_key: Some(secret(SECRET_KEY)),
            region_name: Some("us-east-1".to_owned()),
            session_name: None,
            profile_name: None,
            role_name: None,
            web_identity_token: None,
            timeout: RdsTimeout::SdkDefaults,
        },
        Preset::WebIdentity => RdsCredentialInputs {
            access_key_id: None,
            secret_access_key: None,
            region_name: Some("us-east-1".to_owned()),
            session_name: Some("session-name".to_owned()),
            profile_name: None,
            role_name: Some("arn:aws:iam::123456789012:role/oidc".to_owned()),
            web_identity_token: Some(secret("/token")),
            timeout: RdsTimeout::SdkDefaults,
        },
        Preset::AssumeRole => RdsCredentialInputs {
            access_key_id: None,
            secret_access_key: None,
            region_name: Some("us-east-1".to_owned()),
            session_name: Some("db-session".to_owned()),
            profile_name: None,
            role_name: Some("arn:aws:iam::123456789012:role/db".to_owned()),
            web_identity_token: None,
            timeout: RdsTimeout::SdkDefaults,
        },
    }
}

pub(super) fn endpoint<'input>(
    host: &'input str,
    port: &'input str,
    user: &'input str,
) -> RdsEndpoint<'input> {
    RdsEndpoint { host, port, user }
}
