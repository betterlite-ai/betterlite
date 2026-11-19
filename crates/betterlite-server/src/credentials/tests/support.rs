#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode, header};
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey, UserIdentity,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType};
use betterlite_config::credentials::{CredentialItem, CredentialValues};
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::Payload;
use betterlite_db::resource_repository::credential::CredentialError as RepositoryError;
use betterlite_db::resource_rows::credential::{
    CredentialBase, CredentialItem as CredentialRow, CredentialItemFields,
};
use betterlite_db::stored_cipher::DbCipher;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{
    Algorithm, CipherSettings, DecryptPolicy, FailureLog, FailureValue, OsNonces, StoredValue,
};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use tower::ServiceExt as _;

use crate::auth::auth_identity::AuthContext;
use crate::credentials::lookup::{DeploymentCredentials, DeploymentLookup};
use crate::credentials::registry::CredentialRegistry;
use crate::credentials::router;
use crate::credentials::state::CredentialState;
use crate::credentials::store::{CredentialStore, CredentialStoreError};

pub(crate) const TEST_SIGNING_KEY: &str = "credential-test-signing-key";
pub(crate) const USER_ID: &str = "user-1";

const OPEN: DecryptPolicy = DecryptPolicy {
    log: FailureLog::Debug,
    value: FailureValue::Original,
};

pub(crate) fn cipher_with(signing_key: &str) -> DbCipher {
    DbCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from(signing_key.to_owned())),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

pub(crate) fn cipher() -> Arc<DbCipher> {
    Arc::new(cipher_with(TEST_SIGNING_KEY))
}

pub(crate) fn keyless_cipher() -> Arc<DbCipher> {
    Arc::new(DbCipher {
        settings: CipherSettings {
            signing_key: None,
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    })
}

pub(crate) fn open_text(cipher: &DbCipher, text: &str) -> String {
    let stored = StoredValue::<()>::Text(SecretString::from(text.to_owned()));
    let opened = cipher
        .decrypt_value(stored, "credential_values", OPEN)
        .expect("the value opens");
    let StoredValue::Text(plain) = opened else {
        panic!("a string opens as text");
    };
    plain.expose_secret().to_owned()
}

pub(crate) fn decrypt_text(text: &str) -> String {
    open_text(&cipher(), text)
}

pub(crate) fn sealed(plaintext: &str) -> String {
    let sealed = cipher()
        .encrypt_value(
            StoredValue::<()>::Text(SecretString::from(plaintext.to_owned())),
            None,
        )
        .expect("the test value seals");
    let StoredValue::Text(sealed_text) = sealed else {
        panic!("a string seals as text");
    };
    sealed_text.expose_secret().to_owned()
}

pub(crate) fn text(value: &str) -> JsonData {
    JsonData::String(value.to_owned())
}

pub(crate) fn number(value: i64) -> JsonData {
    JsonData::Number(serde_json::Number::from(value))
}

pub(crate) fn fields(entries: &[(&str, JsonData)]) -> BTreeMap<String, JsonData> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

pub(crate) fn payload(entries: &[(&str, JsonData)]) -> Payload {
    Payload(fields(entries))
}

pub(crate) fn credential_values(entries: &[(&str, JsonData)]) -> CredentialValues {
    CredentialValues::from(payload(entries))
}

pub(crate) fn plain(values: &CredentialValues) -> Payload {
    Payload(
        values
            .0
            .iter()
            .map(|(key, value)| (key.clone(), value.expose_json()))
            .collect(),
    )
}

pub(crate) fn credential(
    name: &str,
    info: &[(&str, JsonData)],
    values: &[(&str, JsonData)],
) -> CredentialItem {
    CredentialItem {
        credential_name: name.to_owned(),
        credential_info: payload(info),
        credential_values: credential_values(values),
    }
}

pub(crate) fn row(credential: &CredentialItem) -> CredentialRow {
    CredentialRow {
        fields: CredentialItemFields {
            base: CredentialBase {
                credential_name: credential.credential_name.clone(),
                credential_info: credential.credential_info.clone(),
            },
            credential_values: plain(&credential.credential_values),
        },
    }
}

pub(crate) fn registry(credentials: Vec<CredentialItem>) -> Arc<CredentialRegistry> {
    let registry = CredentialRegistry::default();
    registry.upsert_credentials(credentials);
    Arc::new(registry)
}

pub(crate) fn create_body(name: &str, values: &str) -> String {
    format!(r#"{{"credential_name":"{name}","credential_info":{{}},"credential_values":{values}}}"#)
}

pub(crate) fn patch_body(name: &str, info: &str, values: &str) -> String {
    format!(
        r#"{{"credential_name":"{name}","credential_info":{info},"credential_values":{values}}}"#
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum StoreFailure {
    Create,
    Duplicate,
    Find,
    Update,
    Delete,
}

#[derive(Default)]
pub(crate) struct FakeStore {
    rows: Mutex<BTreeMap<String, CredentialRow>>,
    failure: Mutex<Option<StoreFailure>>,
    created: Mutex<Vec<(CredentialRow, Option<String>)>>,
    updated: Mutex<Vec<(String, CredentialRow, Option<String>)>>,
    deleted: Mutex<Vec<String>>,
}

impl FakeStore {
    pub(crate) fn seeded(credentials: Vec<CredentialRow>) -> Arc<Self> {
        let store = Self::default();
        for credential in credentials {
            store.insert(credential);
        }
        Arc::new(store)
    }

    pub(crate) fn insert(&self, credential: CredentialRow) {
        self.rows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(credential.fields.base.credential_name.clone(), credential);
    }

    pub(crate) fn fail(&self, failure: StoreFailure) {
        *self.failure.lock().unwrap_or_else(PoisonError::into_inner) = Some(failure);
    }

    pub(crate) fn row(&self, credential_name: &str) -> Option<CredentialRow> {
        self.rows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(credential_name)
            .cloned()
    }

    pub(crate) fn names(&self) -> Vec<String> {
        self.rows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .keys()
            .cloned()
            .collect()
    }

    pub(crate) fn created(&self) -> Vec<(CredentialRow, Option<String>)> {
        self.created
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn updated(&self) -> Vec<(String, CredentialRow, Option<String>)> {
        self.updated
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn deleted(&self) -> Vec<String> {
        self.deleted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn check(&self, wanted: StoreFailure) -> Result<(), CredentialStoreError> {
        let failure = *self.failure.lock().unwrap_or_else(PoisonError::into_inner);
        match failure {
            Some(StoreFailure::Duplicate) if wanted == StoreFailure::Create => {
                Err(CredentialStoreError::Duplicate)
            }
            Some(active) if active == wanted => Err(CredentialStoreError::Repository {
                source: RepositoryError::Table(TableError::MissingRow),
            }),
            Some(_) | None => Ok(()),
        }
    }

    fn push_created(&self, credential: &CredentialRow, actor: Option<&str>) {
        self.created
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((credential.clone(), actor.map(str::to_owned)));
    }

    fn push_updated(&self, credential_name: &str, credential: &CredentialRow, actor: Option<&str>) {
        let record = (
            credential_name.to_owned(),
            credential.clone(),
            actor.map(str::to_owned),
        );
        let mut updated = self.updated.lock().unwrap_or_else(PoisonError::into_inner);
        updated.push(record);
    }

    fn replace(&self, credential_name: &str, credential: &CredentialRow) {
        self.rows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(credential_name);
        self.insert(credential.clone());
    }
}

#[async_trait]
impl CredentialStore for FakeStore {
    async fn create(
        &self,
        credential: &CredentialRow,
        actor: Option<&str>,
    ) -> Result<(), CredentialStoreError> {
        self.check(StoreFailure::Create)?;
        // the unique name constraint a real table carries
        if self.row(&credential.fields.base.credential_name).is_some() {
            return Err(CredentialStoreError::Duplicate);
        }
        self.push_created(credential, actor);
        self.insert(credential.clone());
        Ok(())
    }

    async fn find_by_name(
        &self,
        credential_name: &str,
    ) -> Result<Option<CredentialRow>, CredentialStoreError> {
        self.check(StoreFailure::Find)?;
        Ok(self.row(credential_name))
    }

    async fn update(
        &self,
        credential_name: &str,
        credential: &CredentialRow,
        actor: Option<&str>,
    ) -> Result<(), CredentialStoreError> {
        self.check(StoreFailure::Update)?;
        self.push_updated(credential_name, credential, actor);
        self.replace(credential_name, credential);
        Ok(())
    }

    async fn delete(&self, credential_name: &str) -> Result<bool, CredentialStoreError> {
        self.check(StoreFailure::Delete)?;
        self.deleted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(credential_name.to_owned());
        let removed = self
            .rows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(credential_name);
        Ok(removed.is_some())
    }
}

pub(crate) struct FakeLookup {
    model_id: String,
    model_name: String,
    values: CredentialValues,
}

impl FakeLookup {
    pub(crate) fn serving(
        model_id: &str,
        model_name: &str,
        values: &[(&str, JsonData)],
    ) -> Arc<Self> {
        Arc::new(Self {
            model_id: model_id.to_owned(),
            model_name: model_name.to_owned(),
            values: credential_values(values),
        })
    }
}

impl DeploymentLookup for FakeLookup {
    fn deployment_credentials(&self, model_id: &str) -> Option<DeploymentCredentials> {
        (self.model_id == model_id).then(|| DeploymentCredentials {
            model_name: self.model_name.clone(),
            credential_values: self.values.clone(),
        })
    }
}

pub(crate) fn state(
    registry: &Arc<CredentialRegistry>,
    store: Option<&Arc<FakeStore>>,
    lookup: Option<&Arc<FakeLookup>>,
) -> CredentialState {
    CredentialState {
        registry: Arc::clone(registry),
        cipher: cipher(),
        store: store.map(|fake| -> Arc<dyn CredentialStore> { Arc::<FakeStore>::clone(fake) }),
        lookup: lookup.map(|fake| -> Arc<dyn DeploymentLookup> { Arc::<FakeLookup>::clone(fake) }),
    }
}

pub(crate) fn principal(user_id: Option<&str>) -> Principal {
    Principal {
        principal_type: PrincipalType::ServiceAccount,
        subject: "credential-test".to_owned(),
        issuer: None,
        audience: Vec::new(),
        user: user_id.map(|id| UserIdentity {
            id: id.to_owned(),
            external_id: None,
            user_name: None,
            email: None,
            display_name: None,
        }),
        organization: None,
        teams: Vec::new(),
        project: None,
        end_user: None,
        roles: Vec::new(),
        scopes: Vec::new(),
        auth_method: AuthMethod::ApiKey,
        credential_ref: CredentialRef::default(),
        network: NetworkContext::default(),
        source_key: SourceKey(None),
    }
}

pub(crate) struct Reply {
    pub(crate) status: StatusCode,
    pub(crate) body: String,
}

impl Reply {
    pub(crate) fn json<T>(&self) -> T
    where
        T: DeserializeOwned,
    {
        serde_json::from_str(&self.body).expect("the reply body is the expected JSON")
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct SuccessReply {
    pub(crate) success: bool,
    pub(crate) message: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ReplyCredential {
    #[serde(rename = "credential_name")]
    pub(crate) name: String,
    #[serde(rename = "credential_values")]
    pub(crate) values: Payload,
    #[serde(rename = "credential_info")]
    pub(crate) info: Payload,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ListReply {
    pub(crate) success: bool,
    pub(crate) credentials: Vec<ReplyCredential>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ErrorReply {
    pub(crate) error: ErrorDetail,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ErrorDetail {
    pub(crate) message: String,
    #[serde(rename = "type")]
    pub(crate) error_type: String,
    pub(crate) code: String,
}

pub(crate) async fn call(
    state: CredentialState,
    method: Method,
    uri: &str,
    body: &str,
    caller: Option<&Principal>,
) -> Reply {
    let application = router(state);
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_owned()))
        .expect("the request builds");
    if let Some(principal) = caller {
        request.extensions_mut().insert(AuthContext {
            principal: principal.clone(),
        });
    }
    let response = application
        .oneshot(request)
        .await
        .expect("the router answers");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the body is readable");
    Reply {
        status,
        body: String::from_utf8(bytes.to_vec()).expect("the body is UTF-8"),
    }
}
