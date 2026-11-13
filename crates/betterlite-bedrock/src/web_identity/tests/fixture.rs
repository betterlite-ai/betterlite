use std::collections::{BTreeMap, HashMap};
use std::fmt::Debug;
use std::fmt::Write as _;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, UNIX_EPOCH};

use aws_smithy_async::{
    rt::sleep::{SharedAsyncSleep, TokioSleep},
    time::{SharedTimeSource, StaticTimeSource},
};
use aws_smithy_runtime_api::client::{
    http::{HttpConnector, HttpConnectorFuture, SharedHttpConnector, http_client_fn},
    orchestrator::{HttpRequest, HttpResponse},
    result::ConnectorError,
};
use aws_smithy_runtime_api::http::StatusCode;
use aws_smithy_types::body::SdkBody;
use aws_types::os_shim_internal::Fs;
use betterlite_config::tls::Verification;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use betterlite_secret_managers::secret_lookup::{SecretLookup, manager::ManagerState};
use betterlite_secret_managers::secret_read_dispatch::SecretManagerClient;
use betterlite_secret_managers::secret_read_dispatch::protocols::SyncSecretReader;
use betterlite_secret_managers::secret_read_dispatch::settings::SecretManagerSettings;
use betterlite_secret_managers::secret_types::{KeyManagementSystem, SecretAccessMode};
use secrecy::SecretString;
use tokio::runtime::{Builder, Runtime};
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;
use url::{Url, form_urlencoded};

use crate::aws_endpoints::sts::{StsClientKwargs, StsSettings, sts_client_kwargs};
use crate::aws_settings::sources::NativeSources;
use crate::web_identity::error::WebIdentityError;

pub(super) type Recorder = Arc<Mutex<Vec<Exchange>>>;

#[derive(Debug, Clone, Default)]
pub(super) struct Exchange {
    pub(super) action: String,
    pub(super) form: BTreeMap<String, String>,
    pub(super) access_key: Option<String>,
    pub(super) session_token: Option<String>,
    pub(super) region: Option<String>,
    pub(super) host: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) enum Answer {
    Ok(String),
    Error(u16, String),
}

#[derive(Clone, Debug)]
struct Canned {
    replies: BTreeMap<String, Answer>,
    recorder: Recorder,
}

impl HttpConnector for Canned {
    fn call(&self, request: HttpRequest) -> HttpConnectorFuture {
        let exchange = exchange_of(&request);
        let action = exchange.action.clone();
        self.recorder
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(exchange);
        match self.replies.get(&action) {
            Some(Answer::Ok(body)) => HttpConnectorFuture::ready(Ok(HttpResponse::new(
                status(200_u16),
                SdkBody::from(body.clone()),
            ))),
            Some(Answer::Error(code, body)) => HttpConnectorFuture::ready(Ok(HttpResponse::new(
                status(*code),
                SdkBody::from(body.clone()),
            ))),
            None => {
                let refused = io::Error::other("fixture transport rejected an unexpected call");
                HttpConnectorFuture::ready(Err(ConnectorError::other(Box::new(refused), None)))
            }
        }
    }
}

fn exchange_of(request: &HttpRequest) -> Exchange {
    let request_body = request.body().bytes().unwrap_or_default();
    let form: BTreeMap<String, String> = form_urlencoded::parse(request_body)
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect();
    let authorization = request.headers().get("authorization").unwrap_or_default();
    let scope: Vec<&str> = authorization
        .split("Credential=")
        .nth(1)
        .and_then(|credential| credential.split(',').next())
        .map(|credential| credential.split('/').collect())
        .unwrap_or_default();
    Exchange {
        action: form.get("Action").cloned().unwrap_or_default(),
        form,
        access_key: scope.first().map(|value| (*value).to_owned()),
        session_token: request
            .headers()
            .get("x-amz-security-token")
            .map(str::to_owned),
        region: scope.get(2).map(|value| (*value).to_owned()),
        host: Url::parse(request.uri())
            .ok()
            .and_then(|parsed| parsed.host_str().map(str::to_owned)),
    }
}

fn status(code: u16) -> StatusCode {
    StatusCode::try_from(code).unwrap()
}

pub(super) fn sources(
    environment: &[(&str, &str)],
    files: &[(&str, &str)],
    replies: BTreeMap<String, Answer>,
) -> (NativeSources, Recorder) {
    sources_with_bytes(
        environment,
        &files
            .iter()
            .map(|(path, text)| (*path, text.as_bytes().to_vec()))
            .collect::<Vec<_>>(),
        replies,
    )
}

pub(super) fn sources_with_bytes(
    environment: &[(&str, &str)],
    files: &[(&str, Vec<u8>)],
    replies: BTreeMap<String, Answer>,
) -> (NativeSources, Recorder) {
    let recorder: Recorder = Arc::default();
    let canned = Canned {
        replies,
        recorder: Arc::clone(&recorder),
    };
    let sources = NativeSources {
        environment: environment
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
        filesystem: Fs::from_map(
            files
                .iter()
                .map(|(path, bytes)| ((*path).to_owned(), bytes.clone()))
                .collect::<HashMap<_, _>>(),
        ),
        client: http_client_fn(move |_, _| SharedHttpConnector::new(canned.clone())),
        clock: SharedTimeSource::new(StaticTimeSource::new(UNIX_EPOCH)),
        sleep: SharedAsyncSleep::new(TokioSleep::new()),
        timeout: Duration::from_secs(5),
    };
    (sources, recorder)
}

pub(super) fn kwargs(
    environment: &[(&str, &str)],
    endpoint: Option<&str>,
    region: Option<&str>,
) -> StsClientKwargs {
    let lookup = |name: &str| {
        environment
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| *value)
    };
    let settings = StsSettings {
        endpoint,
        aws_region: lookup("AWS_REGION"),
        aws_default_region: lookup("AWS_DEFAULT_REGION"),
        configured_region: region,
    };
    sts_client_kwargs(&settings, Verification::Enabled).unwrap()
}

struct FixedSecret(Option<SecretString>);

impl SyncSecretReader for FixedSecret {
    fn sync_read_secret(&self, _: &str) -> Option<SecretString> {
        self.0.clone()
    }
}

struct UnusedOidc;

impl OidcHttp for UnusedOidc {
    fn get(&self, _: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: "the fixture has no OIDC transport".to_owned(),
        })
    }
}

impl AzureOidcTokens for UnusedOidc {
    fn token(&self, _: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Ok(None)
    }
}

impl OidcClock for UnusedOidc {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn unix_seconds(&self) -> i64 {
        0
    }
}

pub(super) fn secrets(token: Option<&str>) -> SecretLookup {
    SecretLookup {
        manager: Some(ManagerState {
            client: SecretManagerClient::Vault(Arc::new(FixedSecret(
                token.map(SecretString::from),
            ))),
            settings: Some(SecretManagerSettings::default()),
            system: Some(KeyManagementSystem::HashicorpVault),
            access_mode: SecretAccessMode::ReadOnly,
            hosted_keys: None,
        }),
        oidc_http: Arc::new(UnusedOidc),
        azure_tokens: Arc::new(UnusedOidc),
        clock: Arc::new(UnusedOidc),
        oidc_cache: OidcTokenCache::default(),
    }
}

pub(super) fn runtime() -> Runtime {
    Builder::new_current_thread().enable_all().build().unwrap()
}

pub(super) fn credentials_xml(
    access_key: &str,
    secret_key: &str,
    session_token: &str,
    expiration: &str,
) -> String {
    format!(
        "<Credentials><AccessKeyId>{access_key}</AccessKeyId><SecretAccessKey>{secret_key}</SecretAccessKey><SessionToken>{session_token}</SessionToken><Expiration>{expiration}</Expiration></Credentials>"
    )
}

pub(super) fn web_identity_ok(
    access_key: &str,
    secret_key: &str,
    session_token: &str,
    expiration: &str,
    packed_policy_size: Option<i32>,
) -> Answer {
    let credentials = credentials_xml(access_key, secret_key, session_token, expiration);
    let packed = packed_xml(packed_policy_size);
    Answer::Ok(format!(
        "<AssumeRoleWithWebIdentityResponse><AssumeRoleWithWebIdentityResult>{credentials}{packed}</AssumeRoleWithWebIdentityResult></AssumeRoleWithWebIdentityResponse>"
    ))
}

pub(super) fn assert_missing(error: &WebIdentityError, member: &str) {
    assert!(matches!(
        error,
        WebIdentityError::MissingField(message) if message.ends_with(&format!("no {member}"))
    ));
}

pub(super) fn credentials_without(member: &str) -> String {
    let mut members = vec![
        ("AccessKeyId", "ASIAFIXTURE"),
        ("SecretAccessKey", "fixture-secret"),
        ("SessionToken", "fixture-session"),
        ("Expiration", "2026-01-01T01:00:00Z"),
    ];
    members.retain(|(name, _)| *name != member);
    let mut inner = String::new();
    for (name, value) in &members {
        write!(inner, "<{name}>{value}</{name}>").unwrap();
    }
    format!("<Credentials>{inner}</Credentials>")
}

pub(super) fn web_identity_missing(member: &str) -> Answer {
    Answer::Ok(format!(
        "<AssumeRoleWithWebIdentityResponse><AssumeRoleWithWebIdentityResult>{}</AssumeRoleWithWebIdentityResult></AssumeRoleWithWebIdentityResponse>",
        credentials_without(member)
    ))
}

pub(super) fn assume_role_missing(member: &str) -> Answer {
    Answer::Ok(format!(
        "<AssumeRoleResponse><AssumeRoleResult>{}</AssumeRoleResult></AssumeRoleResponse>",
        credentials_without(member)
    ))
}

pub(super) fn web_identity_without_credentials(packed_policy_size: Option<i32>) -> Answer {
    let packed = packed_xml(packed_policy_size);
    Answer::Ok(format!(
        "<AssumeRoleWithWebIdentityResponse><AssumeRoleWithWebIdentityResult>{packed}</AssumeRoleWithWebIdentityResult></AssumeRoleWithWebIdentityResponse>"
    ))
}

pub(super) fn assume_role_ok(
    access_key: &str,
    secret_key: &str,
    session_token: &str,
    expiration: &str,
) -> Answer {
    let mut body = String::from("<AssumeRoleResponse><AssumeRoleResult>");
    body.push_str(&credentials_xml(
        access_key,
        secret_key,
        session_token,
        expiration,
    ));
    body.push_str("</AssumeRoleResult></AssumeRoleResponse>");
    Answer::Ok(body)
}

pub(super) fn identity_ok(arn: &str) -> Answer {
    Answer::Ok(format!(
        "<GetCallerIdentityResponse><GetCallerIdentityResult><Arn>{arn}</Arn><UserId>AROAFIXTURE:session</UserId><Account>123456789012</Account></GetCallerIdentityResult></GetCallerIdentityResponse>"
    ))
}

pub(super) fn sts_error(code: &str, message: &str, status: u16) -> Answer {
    Answer::Error(
        status,
        format!(
            "<ErrorResponse><Error><Type>Sender</Type><Code>{code}</Code><Message>{message}</Message></Error><RequestId>fixture</RequestId></ErrorResponse>"
        ),
    )
}

pub(super) fn invalid_identity_token() -> Answer {
    sts_error("InvalidIdentityToken", "the token is not valid", 400_u16)
}

pub(super) fn access_denied() -> Answer {
    sts_error("AccessDenied", "not authorized", 403_u16)
}

fn packed_xml(packed_policy_size: Option<i32>) -> String {
    packed_policy_size
        .map(|size| format!("<PackedPolicySize>{size}</PackedPolicySize>"))
        .unwrap_or_default()
}

#[derive(Clone, Default)]
pub(super) struct WarningCapture(Arc<Mutex<Vec<String>>>);

#[derive(Default)]
struct MessageField(String);

impl Visit for MessageField {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.0 = value.to_owned();
        }
    }
}

impl<S: Subscriber> Layer<S> for WarningCapture {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        if *event.metadata().level() != Level::WARN {
            return;
        }
        let mut field = MessageField::default();
        event.record(&mut field);
        let message = field.0.trim_matches('"').to_owned();
        if message.starts_with("The policy size is greater than") {
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(message);
        }
    }
}

pub(super) fn warnings<F>(run: F) -> Vec<String>
where
    F: FnOnce(),
{
    let capture = WarningCapture::default();
    let subscriber = registry().with(capture.clone());
    with_default(subscriber, run);
    capture
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}
