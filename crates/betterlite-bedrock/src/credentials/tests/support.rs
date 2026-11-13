#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::fmt::Debug;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, UNIX_EPOCH};

use aws_smithy_async::rt::sleep::{SharedAsyncSleep, TokioSleep};
use aws_smithy_async::time::{SharedTimeSource, StaticTimeSource};
use aws_smithy_runtime_api::client::http::{
    HttpConnector, HttpConnectorFuture, SharedHttpConnector, http_client_fn,
};
use aws_smithy_runtime_api::client::orchestrator::{HttpRequest, HttpResponse};
use aws_smithy_runtime_api::client::result::ConnectorError;
use aws_smithy_runtime_api::http::StatusCode;
use aws_smithy_types::body::SdkBody;
use aws_types::os_shim_internal::Fs;
use tokio::runtime::{Builder, Runtime};

use crate::aws_endpoints::sts::{StsClientKwargs, StsSettings, sts_client_kwargs};
use crate::aws_settings::credential_cache::CredentialCache;
use crate::aws_settings::sources::NativeSources;
use crate::credentials::AwsCredentialResolver;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::bedrock_aws_settings::AwsS3Settings;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_config::tls::Verification;
use betterlite_context::local_ttl_cache::{
    LocalTtlCache,
    dual::{CacheEvent, Diagnostics, DualLocalCache},
};
use betterlite_redact::secret::Redactor;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::manager::ManagerState;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use betterlite_secret_managers::secret_read_dispatch::SecretManagerClient;
use betterlite_secret_managers::secret_read_dispatch::protocols::SyncSecretReader;
use betterlite_secret_managers::secret_read_dispatch::settings::SecretManagerSettings;
use betterlite_secret_managers::secret_types::{KeyManagementSystem, SecretAccessMode};
use secrecy::SecretString;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;
use url::form_urlencoded;

pub(super) const NOW: u64 = 1_767_225_600;

#[derive(Debug, Clone)]
pub(super) enum Reply {
    Credentials {
        access: &'static str,
        secret: &'static str,
        token: &'static str,
        expiration: &'static str,
    },
    Identity {
        arn: &'static str,
    },
    Error {
        status: u16,
        code: &'static str,
        message: &'static str,
    },
}

#[derive(Clone, Debug)]
pub(super) struct Call {
    pub(super) action: String,
    pub(super) form: BTreeMap<String, String>,
    pub(super) access_key_id: Option<String>,
    pub(super) session_token: Option<String>,
}

impl Call {
    pub(super) fn value(&self, field: &str) -> String {
        self.form.get(field).cloned().unwrap_or_default()
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct Record(Arc<Mutex<Vec<Call>>>);

impl Record {
    pub(super) fn calls(&self) -> Vec<Call> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn count(&self) -> usize {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).len()
    }

    pub(super) fn actions(&self) -> Vec<String> {
        self.calls().into_iter().map(|call| call.action).collect()
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct Transport {
    answers: Arc<Mutex<BTreeMap<String, VecDeque<Reply>>>>,
    pub(super) record: Record,
}

impl Transport {
    pub(super) fn with(answers: &[(&str, Vec<Reply>)]) -> Self {
        Self {
            answers: Arc::new(Mutex::new(
                answers
                    .iter()
                    .map(|(action, replies)| {
                        ((*action).to_owned(), VecDeque::from(replies.clone()))
                    })
                    .collect(),
            )),
            record: Record::default(),
        }
    }
}

fn refused(message: &'static str) -> HttpConnectorFuture {
    HttpConnectorFuture::ready(Err(ConnectorError::other(
        Box::new(io::Error::other(message)),
        None,
    )))
}

fn access_key(request: &HttpRequest) -> Option<String> {
    let authorization = request.headers().get("authorization")?;
    let credential = authorization.split("Credential=").nth(1)?;
    credential.split('/').next().map(str::to_owned)
}

fn form_fields(request: &HttpRequest) -> BTreeMap<String, String> {
    let body = request.body().bytes().unwrap_or_default();
    form_urlencoded::parse(body)
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect()
}

fn credentials_xml(reply: &Reply) -> String {
    let Reply::Credentials {
        access,
        secret,
        token,
        expiration,
    } = reply
    else {
        return String::new();
    };
    format!(
        "<Credentials><AccessKeyId>{access}</AccessKeyId><SecretAccessKey>{secret}</SecretAccessKey><SessionToken>{token}</SessionToken><Expiration>{expiration}</Expiration></Credentials>"
    )
}

fn body(reply: &Reply, action: &str) -> String {
    match reply {
        Reply::Credentials { .. } => {
            if action == "AssumeRoleWithWebIdentity" {
                format!(
                    "<AssumeRoleWithWebIdentityResponse><AssumeRoleWithWebIdentityResult>{}<PackedPolicySize>10</PackedPolicySize></AssumeRoleWithWebIdentityResult></AssumeRoleWithWebIdentityResponse>",
                    credentials_xml(reply)
                )
            } else {
                format!(
                    "<AssumeRoleResponse><AssumeRoleResult>{}</AssumeRoleResult></AssumeRoleResponse>",
                    credentials_xml(reply)
                )
            }
        }
        Reply::Identity { arn } => format!(
            "<GetCallerIdentityResponse><GetCallerIdentityResult><Arn>{arn}</Arn><UserId>fixture</UserId><Account>123456789012</Account></GetCallerIdentityResult></GetCallerIdentityResponse>"
        ),
        Reply::Error { code, message, .. } => format!(
            "<ErrorResponse><Error><Type>Sender</Type><Code>{code}</Code><Message>{message}</Message></Error><RequestId>fixture</RequestId></ErrorResponse>"
        ),
    }
}

impl HttpConnector for Transport {
    fn call(&self, request: HttpRequest) -> HttpConnectorFuture {
        let form = form_fields(&request);
        let Some(action) = form.get("Action").cloned() else {
            return refused("the fixture request has no action");
        };
        self.record
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Call {
                action: action.clone(),
                form,
                access_key_id: access_key(&request),
                session_token: request
                    .headers()
                    .get("x-amz-security-token")
                    .map(str::to_owned),
            });
        let Some(reply) = self
            .answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_mut(&action)
            .and_then(VecDeque::pop_front)
        else {
            return refused("the fixture has no answer for this action");
        };
        let status = if let Reply::Error { status, .. } = &reply {
            *status
        } else {
            200
        };
        let Ok(status_code) = StatusCode::try_from(status) else {
            return refused("the fixture answer status is invalid");
        };
        HttpConnectorFuture::ready(Ok(HttpResponse::new(
            status_code,
            SdkBody::from(body(&reply, &action)),
        )))
    }
}

pub(super) fn load(params: &str, environment: &[(&str, &str)]) -> (AwsSettings, AwsS3Settings) {
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    let source = format!(
        "model_list:\n- model_name: fixture\n  litellm_params: {{model: bedrock/fixture, {params}}}\n"
    );
    let config = ConfigLoader { lookup }.load(&source).unwrap();
    let settings = config
        .sections
        .bedrock_settings
        .deployments
        .into_iter()
        .next()
        .unwrap();
    let s3 = config
        .sections
        .bedrock_aws_settings
        .deployments
        .into_iter()
        .next()
        .unwrap();
    (settings, s3)
}

pub(super) fn settings(params: &str, environment: &[(&str, &str)]) -> AwsSettings {
    load(params, environment).0
}

pub(super) fn sts_kwargs(environment: &[(&str, &str)]) -> StsClientKwargs {
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    let settings = StsSettings {
        endpoint: None,
        aws_region: lookup.get("AWS_REGION").map(String::as_str),
        aws_default_region: lookup.get("AWS_DEFAULT_REGION").map(String::as_str),
        configured_region: None,
    };
    sts_client_kwargs(&settings, Verification::Enabled).unwrap()
}

pub(super) fn cache(events: Arc<Mutex<Vec<String>>>) -> CredentialCache {
    let memory = LocalTtlCache::with_clock(
        Some(1000),
        None,
        None,
        1024,
        Arc::new(|| f64::from(u32::try_from(NOW).unwrap())),
    )
    .unwrap();
    CredentialCache::new(DualLocalCache {
        memory: Arc::new(Mutex::new(memory)),
        default_in_memory_ttl: None,
        diagnostics: Diagnostics {
            redactor: Arc::new(Redactor::new(16).unwrap()),
            emit: Arc::new(move |event: CacheEvent| {
                events
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(event.message);
            }),
        },
    })
    .unwrap()
}

struct Ports(Option<SecretString>);

impl SyncSecretReader for Ports {
    fn sync_read_secret(&self, _: &str) -> Option<SecretString> {
        self.0.clone()
    }
}

impl OidcHttp for Ports {
    fn get(&self, _: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: "the fixture has no OIDC transport".to_owned(),
        })
    }
}

impl AzureOidcTokens for Ports {
    fn token(&self, _: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Ok(None)
    }
}

impl OidcClock for Ports {
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
            client: SecretManagerClient::Vault(Arc::new(Ports(
                token.map(|value| SecretString::from(value.to_owned())),
            ))),
            settings: Some(SecretManagerSettings::default()),
            system: Some(KeyManagementSystem::HashicorpVault),
            access_mode: SecretAccessMode::ReadOnly,
            hosted_keys: None,
        }),
        oidc_http: Arc::new(Ports(None)),
        azure_tokens: Arc::new(Ports(None)),
        clock: Arc::new(Ports(None)),
        oidc_cache: OidcTokenCache::default(),
    }
}

pub(super) struct Fixture {
    pub(super) cache: CredentialCache,
    pub(super) native: NativeSources,
    pub(super) secrets: SecretLookup,
    pub(super) sts: StsClientKwargs,
    pub(super) runtime: Runtime,
    pub(super) events: Arc<Mutex<Vec<String>>>,
}

impl Fixture {
    pub(super) fn new(
        environment: &[(&str, &str)],
        files: &[(&str, &str)],
        transport: &Transport,
        token: Option<&str>,
    ) -> Self {
        let connector = transport.clone();
        let events: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        Self {
            cache: cache(Arc::clone(&events)),
            native: NativeSources {
                environment: environment
                    .iter()
                    .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                    .collect(),
                filesystem: Fs::from_slice(files),
                client: http_client_fn(move |_, _| SharedHttpConnector::new(connector.clone())),
                clock: SharedTimeSource::new(StaticTimeSource::new(
                    UNIX_EPOCH.checked_add(Duration::from_secs(NOW)).unwrap(),
                )),
                sleep: SharedAsyncSleep::new(TokioSleep::new()),
                timeout: Duration::from_secs(5),
            },
            secrets: secrets(token),
            sts: sts_kwargs(environment),
            runtime: Builder::new_current_thread().enable_all().build().unwrap(),
            events,
        }
    }

    pub(super) fn resolver(&self) -> AwsCredentialResolver<'_> {
        AwsCredentialResolver {
            cache: &self.cache,
            native: &self.native,
            secrets: &self.secrets,
            sts: &self.sts,
            max_policy_size: 75,
        }
    }

    pub(super) fn cache_events(&self) -> Vec<String> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Clone, Default)]
pub(super) struct Logs(Arc<Mutex<Vec<(Level, String)>>>);

#[derive(Default)]
struct Fields(String);

impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<S: Subscriber> Layer<S> for Logs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let level = *event.metadata().level();
        if level != Level::WARN && level != Level::ERROR {
            return;
        }
        let mut fields = Fields::default();
        event.record(&mut fields);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((level, fields.0.trim_matches('"').to_owned()));
    }
}

impl Logs {
    pub(super) fn entries(&self) -> Vec<(Level, String)> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

pub(super) fn logged<F, R>(run: F) -> (R, Vec<(Level, String)>)
where
    F: FnOnce() -> R,
{
    let logs = Logs::default();
    let subscriber = registry().with(logs.clone());
    let value = with_default(subscriber, run);
    (value, logs.entries())
}
