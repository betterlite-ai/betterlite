use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex};
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
use betterlite_bedrock::aws_endpoints::sts::{StsClientKwargs, StsSettings, sts_client_kwargs};
use betterlite_bedrock::aws_settings::credential_cache::CredentialCache;
use betterlite_bedrock::aws_settings::sources::NativeSources;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_config::tls::Verification;
use betterlite_context::local_ttl_cache::{
    LocalTtlCache,
    dual::{Diagnostics, DualLocalCache},
};
use betterlite_redact::secret::Redactor;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use secrecy::SecretString;
use url::form_urlencoded;

use super::failure;
use crate::helpers::HelperError;

#[derive(Clone, Debug, Default)]
pub(super) struct Transport(Arc<Mutex<Vec<String>>>);

impl Transport {
    fn push(&self, action: String) {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(action);
    }

    pub(super) fn actions(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl HttpConnector for Transport {
    fn call(&self, request: HttpRequest) -> HttpConnectorFuture {
        let body = request.body().bytes().unwrap_or_default();
        let form: BTreeMap<String, String> = form_urlencoded::parse(body)
            .map(|(name, value)| (name.into_owned(), value.into_owned()))
            .collect();
        let Some(action) = form.get("Action").cloned() else {
            return refused("the fixture request has no action");
        };
        let Some(answer) = answer(&action) else {
            return refused("the fixture has no answer for this action");
        };
        self.push(action);
        let Ok(status) = StatusCode::try_from(200_u16) else {
            return refused("the fixture status is invalid");
        };
        HttpConnectorFuture::ready(Ok(HttpResponse::new(
            status,
            SdkBody::from(answer.to_owned()),
        )))
    }
}

fn answer(action: &str) -> Option<&'static str> {
    match action {
        "GetCallerIdentity" => Some(
            "<GetCallerIdentityResponse><GetCallerIdentityResult><Arn>arn:aws:iam::999999999999:role/other</Arn><UserId>fixture</UserId><Account>999999999999</Account></GetCallerIdentityResult></GetCallerIdentityResponse>",
        ),
        "AssumeRole" => Some(
            "<AssumeRoleResponse><AssumeRoleResult><Credentials><AccessKeyId>sts-access</AccessKeyId><SecretAccessKey>sts-secret</SecretAccessKey><SessionToken>sts-token</SessionToken><Expiration>2030-01-01T00:00:00Z</Expiration></Credentials></AssumeRoleResult></AssumeRoleResponse>",
        ),
        "AssumeRoleWithWebIdentity" => Some(
            "<AssumeRoleWithWebIdentityResponse><AssumeRoleWithWebIdentityResult><Credentials><AccessKeyId>sts-access</AccessKeyId><SecretAccessKey>sts-secret</SecretAccessKey><SessionToken>sts-token</SessionToken><Expiration>2030-01-01T00:00:00Z</Expiration></Credentials><PackedPolicySize>0</PackedPolicySize></AssumeRoleWithWebIdentityResult></AssumeRoleWithWebIdentityResponse>",
        ),
        _ => None,
    }
}

fn refused(message: &'static str) -> HttpConnectorFuture {
    HttpConnectorFuture::ready(Err(ConnectorError::other(
        Box::new(io::Error::other(message)),
        None,
    )))
}

struct NoPorts;

impl OidcHttp for NoPorts {
    fn get(&self, _: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: "the fixture has no OIDC transport".to_owned(),
        })
    }
}

impl AzureOidcTokens for NoPorts {
    fn token(&self, _: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Ok(None)
    }
}

impl OidcClock for NoPorts {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn unix_seconds(&self) -> i64 {
        0
    }
}

pub(super) fn secrets() -> SecretLookup {
    SecretLookup {
        manager: None,
        oidc_http: Arc::new(NoPorts),
        azure_tokens: Arc::new(NoPorts),
        clock: Arc::new(NoPorts),
        oidc_cache: OidcTokenCache::default(),
    }
}

pub(super) fn clocked_memory(now: u32) -> Result<Arc<Mutex<LocalTtlCache>>, Box<HelperError>> {
    let moment = f64::from(now);
    let memory = LocalTtlCache::with_clock(Some(1000), None, None, 1024, Arc::new(move || moment))
        .map_err(failure)?;
    Ok(Arc::new(Mutex::new(memory)))
}

pub(super) fn fixture_cache(
    memory: &Arc<Mutex<LocalTtlCache>>,
) -> Result<CredentialCache, Box<HelperError>> {
    CredentialCache::new(DualLocalCache {
        memory: Arc::clone(memory),
        default_in_memory_ttl: None,
        diagnostics: Diagnostics {
            redactor: Arc::new(Redactor::new(16).map_err(failure)?),
            emit: Arc::new(|_| {}),
        },
    })
    .map_err(failure)
}

pub(super) fn native_sources(
    environment: &BTreeMap<String, String>,
    files: &BTreeMap<String, String>,
    transport: &Transport,
) -> NativeSources {
    let mounted: Vec<(&str, &str)> = files
        .iter()
        .map(|(path, text)| (path.as_str(), text.as_str()))
        .collect();
    let connector = transport.clone();
    NativeSources {
        environment: environment.clone(),
        filesystem: Fs::from_slice(&mounted),
        client: http_client_fn(move |_, _| SharedHttpConnector::new(connector.clone())),
        clock: SharedTimeSource::new(StaticTimeSource::new(UNIX_EPOCH)),
        sleep: SharedAsyncSleep::new(TokioSleep::new()),
        timeout: Duration::from_secs(5),
    }
}

pub(super) fn sts_kwargs(
    settings: &AwsSettings,
    environment: &BTreeMap<String, String>,
) -> Result<StsClientKwargs, Box<HelperError>> {
    sts_client_kwargs(
        &StsSettings {
            endpoint: settings.sts_endpoint.as_deref(),
            aws_region: environment.get("AWS_REGION").map(String::as_str),
            aws_default_region: environment.get("AWS_DEFAULT_REGION").map(String::as_str),
            configured_region: settings.region_name.as_deref(),
        },
        Verification::Enabled,
    )
    .map_err(failure)
}
