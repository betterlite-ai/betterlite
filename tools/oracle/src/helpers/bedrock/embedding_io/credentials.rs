use std::collections::BTreeMap;
use std::future::ready;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::bedrock_embedding_io::credentials::{
    CredentialRequest as LoadRequest, ManagedRegionWorker, load_credentials,
};
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_config::sections::bedrock_settings::{AwsSettings, SessionTagValue};
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use secrecy::{ExposeSecret, SecretString};
use serde_json::value::RawValue;
use serde_json::{Value, json};

use super::failure::Failure;
use super::input::{CredentialRequest as Case, Keys};
use super::runtime;

pub(crate) fn run(case: &Case) -> Result<Value, Failure> {
    let mut parameters = parameters(&case.optional_params)?;
    let bearer = case.bearer_token.as_deref().map(SecretString::from);
    let source = Source::new(&case.credentials);
    let secrets = secrets();
    let region_worker = ManagedRegionWorker::new()?;
    let loaded = runtime::block_on(load_credentials(
        LoadRequest {
            optional_params: &mut parameters,
            environment: &case.environment,
            bearer_token: bearer.as_ref(),
        },
        &secrets,
        &region_worker,
        |settings: &AwsSettings, region: &str| {
            source.record(settings, region);
            ready(Ok::<Credentials, CredentialError>(
                source.credentials.clone(),
            ))
        },
    ))??;
    let calls = source.calls();
    let auth = match calls.first() {
        Some((settings, _)) => Some(auth_object(settings)?),
        None => None,
    };
    Ok(json!({
        "region": loaded.region,
        "credentials": loaded.credentials.as_ref().map(credentials_object).transpose()?,
        "resolved": calls.len(),
        "resolve_auth": auth,
        "resolve_region": calls.first().map(|(_, region)| region.clone()),
        "remaining": remaining(&parameters)?,
    }))
}

fn parameters(values: &Object) -> Result<BTreeMap<String, Box<RawValue>>, Failure> {
    let mut parameters = BTreeMap::new();
    for (name, value) in values {
        parameters.insert(
            name.clone(),
            RawValue::from_string(serde_json::to_string(value)?)?,
        );
    }
    Ok(parameters)
}

fn remaining(parameters: &BTreeMap<String, Box<RawValue>>) -> Result<Object, Failure> {
    let mut object = Object::new();
    for (name, raw) in parameters {
        object.insert(name.clone(), serde_json::from_str::<Json>(raw.get())?);
    }
    Ok(object)
}

fn credentials_object(credentials: &Credentials) -> Result<Object, Failure> {
    let mut object = Object::new();
    object.insert(
        "access_key_id".to_owned(),
        serde_json::from_value(json!(credentials.access_key_id.expose_secret()))?,
    );
    object.insert(
        "secret_access_key".to_owned(),
        serde_json::from_value(json!(credentials.secret_access_key.expose_secret()))?,
    );
    object.insert(
        "session_token".to_owned(),
        serde_json::from_value(json!(
            credentials
                .session_token
                .as_ref()
                .map(ExposeSecret::expose_secret)
        ))?,
    );
    Ok(object)
}

fn auth_object(settings: &AwsSettings) -> Result<Object, Failure> {
    let mut object = Object::new();
    object.insert(
        "aws_access_key_id".to_owned(),
        serde_json::from_value(json!(
            settings
                .access_key_id
                .as_ref()
                .map(ExposeSecret::expose_secret)
        ))?,
    );
    object.insert(
        "aws_secret_access_key".to_owned(),
        serde_json::from_value(json!(
            settings
                .secret_access_key
                .as_ref()
                .map(ExposeSecret::expose_secret)
        ))?,
    );
    object.insert(
        "aws_session_token".to_owned(),
        serde_json::from_value(json!(
            settings
                .session_token
                .as_ref()
                .map(ExposeSecret::expose_secret)
        ))?,
    );
    object.insert(
        "aws_session_name".to_owned(),
        serde_json::from_value(json!(settings.session_name.as_deref()))?,
    );
    object.insert(
        "aws_profile_name".to_owned(),
        serde_json::from_value(json!(settings.profile_name.as_deref()))?,
    );
    object.insert(
        "aws_role_name".to_owned(),
        serde_json::from_value(json!(settings.role_name.as_deref()))?,
    );
    object.insert(
        "aws_web_identity_token".to_owned(),
        serde_json::from_value(json!(
            settings
                .web_identity_token
                .as_ref()
                .map(ExposeSecret::expose_secret)
        ))?,
    );
    object.insert(
        "aws_sts_endpoint".to_owned(),
        serde_json::from_value(json!(settings.sts_endpoint.as_deref()))?,
    );
    object.insert(
        "aws_external_id".to_owned(),
        serde_json::from_value(json!(settings.external_id.as_deref()))?,
    );
    object.insert(
        "aws_session_tags".to_owned(),
        tags_value(settings.session_tags.as_ref())?,
    );
    Ok(object)
}

fn tags_value(tags: Option<&SessionTagValue>) -> Result<Json, Failure> {
    match tags {
        Some(tags) => Ok(serde_json::from_value(serde_json::to_value(tags)?)?),
        None => Ok(Json::Null),
    }
}

struct Source {
    credentials: Credentials,
    calls: Mutex<Vec<(AwsSettings, String)>>,
}

impl Source {
    fn new(keys: &Keys) -> Self {
        Self {
            credentials: Credentials {
                access_key_id: SecretString::from(keys.access_key_id.clone()),
                secret_access_key: SecretString::from(keys.secret_access_key.clone()),
                session_token: keys.session_token.clone().map(SecretString::from),
            },
            calls: Mutex::new(Vec::new()),
        }
    }

    fn record(&self, settings: &AwsSettings, region: &str) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((settings.clone(), region.to_owned()));
    }

    fn calls(&self) -> Vec<(AwsSettings, String)> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

struct NoPorts;

impl OidcHttp for NoPorts {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: "the fixture has no OIDC transport".to_owned(),
        })
    }
}

impl AzureOidcTokens for NoPorts {
    fn token(&self, _scope: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Err(AzureOidcError::Provider {
            message: "the fixture has no Azure token provider".to_owned(),
        })
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

fn secrets() -> SecretLookup {
    SecretLookup {
        manager: None,
        oidc_http: Arc::new(NoPorts),
        azure_tokens: Arc::new(NoPorts),
        clock: Arc::new(NoPorts),
        oidc_cache: OidcTokenCache::default(),
    }
}
