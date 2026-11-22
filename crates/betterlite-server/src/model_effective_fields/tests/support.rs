use std::sync::Mutex;

use async_trait::async_trait;
use betterlite_db::identity_rows::Payload;
use betterlite_db::resource_rows::credential::{
    CredentialBase, CredentialItem as CredentialRow, CredentialItemFields,
};
use betterlite_db::stored_cipher::DbCipher;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces, StoredValue};
use secrecy::{ExposeSecret as _, SecretString};

use crate::credentials::store::{CredentialStore, CredentialStoreError};
use crate::jobs::ptu::attribution::PtuAttribution;

pub(super) fn cipher() -> DbCipher {
    DbCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from("b1213-salt".to_owned())),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

pub(super) fn sealed(text: &str) -> String {
    let value = StoredValue::<()>::Text(SecretString::from(text.to_owned()));
    match cipher().encrypt_value(value, None).unwrap() {
        StoredValue::Text(sealed) => sealed.expose_secret().to_owned(),
        StoredValue::Other(()) => panic!("text seals to text"),
    }
}

pub(super) fn object(text: &str) -> JsonObject {
    serde_json::from_str(text).unwrap()
}

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

pub(super) struct Attribution(pub(super) bool);

impl PtuAttribution for Attribution {
    fn enabled(&self) -> bool {
        self.0
    }
}

#[derive(Default)]
pub(super) struct Store {
    pub(super) names: Vec<String>,
    pub(super) broken: bool,
    pub(super) lookups: Mutex<Vec<String>>,
}

#[async_trait]
impl CredentialStore for Store {
    async fn create(
        &self,
        _credential: &CredentialRow,
        _actor: Option<&str>,
    ) -> Result<(), CredentialStoreError> {
        Ok(())
    }

    async fn find_by_name(
        &self,
        credential_name: &str,
    ) -> Result<Option<CredentialRow>, CredentialStoreError> {
        self.lookups
            .lock()
            .unwrap()
            .push(credential_name.to_owned());
        if self.broken {
            return Err(CredentialStoreError::Duplicate);
        }
        let found = self.names.iter().any(|name| name == credential_name);
        Ok(found.then(|| CredentialRow {
            fields: CredentialItemFields {
                base: CredentialBase {
                    credential_name: credential_name.to_owned(),
                    credential_info: Payload::default(),
                },
                credential_values: Payload::default(),
            },
        }))
    }

    async fn update(
        &self,
        _credential_name: &str,
        _credential: &CredentialRow,
        _actor: Option<&str>,
    ) -> Result<(), CredentialStoreError> {
        Ok(())
    }

    async fn delete(&self, _credential_name: &str) -> Result<bool, CredentialStoreError> {
        Ok(false)
    }
}
