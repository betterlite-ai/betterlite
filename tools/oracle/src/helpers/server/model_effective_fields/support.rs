use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_db::identity_rows::Payload;
use betterlite_db::resource_rows::credential::{
    CredentialBase, CredentialItem as CredentialRow, CredentialItemFields,
};
use betterlite_server::credentials::store::{CredentialStore, CredentialStoreError};
use betterlite_server::jobs::ptu::attribution::PtuAttribution;
use betterlite_server::model_effective_fields::error::ModelFieldError;
use serde_json::{Value, json};

pub(super) struct Attribution(pub(super) bool);

impl PtuAttribution for Attribution {
    fn enabled(&self) -> bool {
        self.0
    }
}

// the credentials table holds the names a case lists and records each name it is asked for
pub(super) struct FakeCredentials {
    pub(super) names: Vec<String>,
    pub(super) lookups: Mutex<Vec<String>>,
}

impl FakeCredentials {
    pub(super) fn lookups(&self) -> Vec<String> {
        self.lookups
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl CredentialStore for FakeCredentials {
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
            .unwrap_or_else(PoisonError::into_inner)
            .push(credential_name.to_owned());
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

// a ProxyException carries its status as the code, and LiteLLM's bare HTTPException carries neither type nor code
pub(super) fn refusal(outcome: Result<(), ModelFieldError>) -> Value {
    let Err(error) = outcome else {
        return Value::Null;
    };
    let refused = error.refusal();
    let error_type = refused.error_type.map(|kind| kind.to_string());
    let code = error_type.as_ref().map(|_kind| refused.status.to_string());
    json!({
        "status": refused.status,
        "type": error_type,
        "param": refused.param,
        "code": code,
    })
}
