#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;

use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::body::to_bytes;
use axum::http::Request;
use axum::http::StatusCode;
use betterlite_auth::auth_identity::records::SourceKey;
use betterlite_auth::auth_identity::records::{CredentialRef, NetworkContext, Principal};
use betterlite_auth::auth_identity::roles::AuthMethod;
use betterlite_auth::auth_identity::roles::PrincipalType;
use betterlite_auth::auth_identity::roles::Role;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_redis::redis_connection_settings::environment::EnvironmentLookup;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings};
use secrecy::SecretString;
use serde_json::{Map, Value};
use tower::ServiceExt as _;

use crate::auth::auth_identity::AuthContext;

use super::stored::CacheConfigStore;

pub(crate) fn parse(text: &str) -> Value {
    serde_json::from_str(text).expect("valid JSON fixture")
}

pub(crate) fn obj(text: &str) -> Map<String, Value> {
    parse(text).as_object().expect("object fixture").clone()
}

pub(crate) fn environment(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(crate) fn lookup(entries: &[(&str, &str)]) -> Arc<dyn EnvironmentLookup + Send + Sync> {
    Arc::new(environment(entries))
}

pub(crate) fn cipher_settings(key: Option<&str>) -> CipherSettings {
    CipherSettings {
        signing_key: key.map(SecretString::from),
        algorithm: Algorithm::XSalsa20Poly1305,
    }
}

pub(crate) struct SwitchableEnvironment {
    values: Mutex<BTreeMap<String, String>>,
}

impl SwitchableEnvironment {
    pub(crate) fn new(entries: &[(&str, &str)]) -> Self {
        Self {
            values: Mutex::new(environment(entries)),
        }
    }

    pub(crate) fn replace(&self, entries: &[(&str, &str)]) {
        let mut values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        *values = environment(entries);
    }
}

impl EnvironmentLookup for SwitchableEnvironment {
    fn lookup(&self, name: &str) -> Option<String> {
        self.values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(name)
            .cloned()
    }
}

pub(crate) enum StoreReply {
    Settings(Value),
    Missing,
    Failing,
}

pub(crate) struct FakeCacheConfigStore {
    reply: StoreReply,
}

impl FakeCacheConfigStore {
    pub(crate) fn new(reply: StoreReply) -> Self {
        Self { reply }
    }
}

#[async_trait]
impl CacheConfigStore for FakeCacheConfigStore {
    async fn stored_cache_settings(&self) -> Result<Option<Value>, TableError> {
        match &self.reply {
            StoreReply::Settings(raw) => Ok(Some(raw.clone())),
            StoreReply::Missing => Ok(None),
            StoreReply::Failing => Err(TableError::MissingRow),
        }
    }
}

pub(crate) fn caller(subject: &str, roles: Vec<Role>) -> AuthContext {
    AuthContext {
        principal: Principal {
            principal_type: PrincipalType::ServiceAccount,
            subject: subject.to_owned(),
            issuer: None,
            audience: Vec::new(),
            user: None,
            organization: None,
            teams: Vec::new(),
            project: None,
            end_user: None,
            roles,
            scopes: Vec::new(),
            auth_method: AuthMethod::ApiKey,
            credential_ref: CredentialRef::default(),
            network: NetworkContext::default(),
            source_key: SourceKey::default(),
        },
    }
}

pub(crate) async fn get(application: Router, auth: Option<AuthContext>) -> (StatusCode, String) {
    let mut request = Request::get("/cache/settings")
        .body(Body::empty())
        .expect("a GET request");
    if let Some(context) = auth {
        request.extensions_mut().insert(context);
    }
    let response = application
        .oneshot(request)
        .await
        .expect("the route answers");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the body is readable");
    (
        status,
        String::from_utf8(bytes.to_vec()).expect("the body is UTF-8"),
    )
}
