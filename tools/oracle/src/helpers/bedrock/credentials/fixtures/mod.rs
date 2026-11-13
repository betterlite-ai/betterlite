use std::collections::{BTreeMap, VecDeque};
use std::io::Error;
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
use betterlite_bedrock::aws_endpoints::error::AwsEndpointError;
use betterlite_bedrock::aws_endpoints::sts::{StsClientKwargs, StsSettings, sts_client_kwargs};
use betterlite_bedrock::aws_settings::credential_cache::CredentialCache;
use betterlite_bedrock::aws_settings::sources::NativeSources;
use betterlite_config::tls::Verification;
use betterlite_context::local_ttl_cache::{
    LocalTtlCache,
    dual::{Diagnostics, DualLocalCache},
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
use serde::Serialize;
use serde_json::Value;
use url::form_urlencoded;

use super::input::{Answers, Case, DEFAULT_NOW, Reply};
use super::ops::failure;
use crate::helpers::HelperError;

#[derive(Clone, Debug, Serialize)]
pub(super) struct Call {
    pub action: String,
    pub form: BTreeMap<String, String>,
    pub access_key_id: Option<String>,
    pub session_token: Option<String>,
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
}

#[derive(Clone, Debug, Default)]
pub(super) struct Transport {
    pub(super) answers: Arc<Mutex<BTreeMap<String, VecDeque<Reply>>>>,
    pub(super) record: Record,
}

impl Transport {
    pub(super) fn with(answers: &Answers) -> Self {
        Self {
            answers: Arc::new(Mutex::new(BTreeMap::from([
                (
                    "AssumeRole".to_owned(),
                    VecDeque::from(answers.assume_role.clone()),
                ),
                (
                    "AssumeRoleWithWebIdentity".to_owned(),
                    VecDeque::from(answers.assume_role_with_web_identity.clone()),
                ),
                (
                    "GetCallerIdentity".to_owned(),
                    VecDeque::from(answers.get_caller_identity.clone()),
                ),
            ]))),
            record: Record::default(),
        }
    }
}

fn refused(message: &'static str) -> HttpConnectorFuture {
    HttpConnectorFuture::ready(Err(ConnectorError::other(
        Box::new(Error::other(message)),
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
    let Reply::Credentials(credentials) = reply else {
        return String::new();
    };
    let packed = credentials
        .packed_policy_size
        .map_or_else(String::new, |size| {
            format!("<PackedPolicySize>{size}</PackedPolicySize>")
        });
    format!(
        "<Credentials><AccessKeyId>{}</AccessKeyId><SecretAccessKey>{}</SecretAccessKey><SessionToken>{}</SessionToken><Expiration>{}</Expiration></Credentials>{packed}",
        credentials.access_key_id,
        credentials.secret_access_key,
        credentials.session_token,
        credentials.expiration
    )
}

fn body(reply: &Reply, action: &str) -> String {
    match reply {
        Reply::Credentials(_) => {
            if action == "AssumeRoleWithWebIdentity" {
                format!(
                    "<AssumeRoleWithWebIdentityResponse><AssumeRoleWithWebIdentityResult>{}</AssumeRoleWithWebIdentityResult></AssumeRoleWithWebIdentityResponse>",
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

pub(super) fn native(case: &Case, transport: &Transport) -> NativeSources {
    let connector = transport.clone();
    let files: Vec<(&str, &str)> = case
        .files
        .iter()
        .map(|(path, content)| (path.as_str(), content.as_str()))
        .collect();
    let clock = UNIX_EPOCH
        .checked_add(Duration::from_secs(
            u64::try_from(case.now.unwrap_or(DEFAULT_NOW)).unwrap_or_default(),
        ))
        .unwrap_or(UNIX_EPOCH);
    NativeSources {
        environment: case.environment.clone(),
        filesystem: Fs::from_slice(&files),
        client: http_client_fn(move |_, _| SharedHttpConnector::new(connector.clone())),
        clock: SharedTimeSource::new(StaticTimeSource::new(clock)),
        sleep: SharedAsyncSleep::new(TokioSleep::new()),
        timeout: Duration::from_secs(5),
    }
}

pub(super) fn kwargs(
    environment: &BTreeMap<String, String>,
    params: &BTreeMap<String, Value>,
) -> Result<StsClientKwargs, AwsEndpointError> {
    let settings = StsSettings {
        endpoint: params.get("aws_sts_endpoint").and_then(Value::as_str),
        aws_region: environment.get("AWS_REGION").map(String::as_str),
        aws_default_region: environment.get("AWS_DEFAULT_REGION").map(String::as_str),
        configured_region: params.get("aws_region_name").and_then(Value::as_str),
    };
    sts_client_kwargs(&settings, Verification::Enabled)
}

pub(super) fn cache(now: i64) -> Result<CredentialCache, Box<HelperError>> {
    let seconds = f64::from(u32::try_from(now).unwrap_or_default());
    let memory = LocalTtlCache::with_clock(Some(1000), None, None, 1024, Arc::new(move || seconds))
        .map_err(failure)?;
    CredentialCache::new(DualLocalCache {
        memory: Arc::new(Mutex::new(memory)),
        default_in_memory_ttl: None,
        diagnostics: Diagnostics {
            redactor: Arc::new(Redactor::new(16).map_err(failure)?),
            emit: Arc::new(|_| {}),
        },
    })
    .map_err(failure)
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
