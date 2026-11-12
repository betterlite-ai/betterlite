use std::fmt;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

use crate::call_setup::RequestMap;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CredentialError {
    #[error("the {field} value must be a string")]
    FieldType { field: &'static str },
}

pub struct Credential {
    pub credential_name: String,
    pub credential_values: IndexMap<String, SecretString>,
}

// Values stay inside SecretString, so a printed credential names only the fields it carries
impl fmt::Debug for Credential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credential")
            .field("credential_name", &self.credential_name)
            .field(
                "credential_values",
                &self.credential_values.keys().collect::<Vec<_>>(),
            )
            .finish()
    }
}

pub trait CredentialLookup: Send + Sync {
    fn find_credential(&self, credential_name: &str) -> Option<&Credential>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialWarning {
    pub credential_name: String,
}

pub fn load_credentials_from_list(
    request: &mut RequestMap,
    credentials: &dyn CredentialLookup,
) -> Result<Option<CredentialWarning>, CredentialError> {
    let name = match request.get("litellm_credential_name") {
        None | Some(Json::Null) => return Ok(None),
        Some(Json::Text(name)) => name.clone(),
        Some(_) => {
            return Err(CredentialError::FieldType {
                field: "litellm_credential_name",
            });
        }
    };
    let Some(credential) = credentials.find_credential(&name) else {
        return Ok(Some(CredentialWarning {
            credential_name: name,
        }));
    };
    for (key, value) in &credential.credential_values {
        if !request.contains_key(key) {
            request.insert(key.clone(), Json::Text(value.expose_secret().to_owned()));
        }
    }
    Ok(None)
}
