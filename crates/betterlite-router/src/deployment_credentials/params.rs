use std::fmt;

use betterlite_auth::auth_model_lists::parameters::CREDENTIAL_FIELDS;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use secrecy::SecretString;

use crate::deployment_credentials::error::CredentialError;

// pydantic declares vertex_credentials as str | dict and the two tag fields as lists, never text.
const TEXT_OR_OBJECT: [&str; 1] = ["vertex_credentials"];
const LIST_ONLY: [&str; 2] = ["aws_session_tags", "bedrock_tags"];

// CredentialLiteLLMParams.model_dump(exclude_none=True): the declared fields in declared order, nulls left out.
#[derive(Clone, Default)]
pub struct CredentialParams {
    pub fields: JsonObject,
}

// A credential value is a secret, so Debug names the fields and prints none of the values.
impl fmt::Debug for CredentialParams {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map()
            .entries(self.fields.keys().map(|name| (name, "[REDACTED]")))
            .finish()
    }
}

impl CredentialParams {
    // CredentialLiteLLMParams.model_validate(...): declared fields only, null fields excluded, other keys ignored.
    pub fn project(litellm_params: &JsonObject) -> Result<Self, CredentialError> {
        let mut fields = JsonObject::new();
        for field in CREDENTIAL_FIELDS {
            let Some(value) = litellm_params.get(field) else {
                continue;
            };
            let typed = match value {
                Json::Null => continue,
                Json::Text(_) if LIST_ONLY.contains(&field) => Err("a list"),
                Json::Text(_) => Ok(()),
                Json::Object(_) if TEXT_OR_OBJECT.contains(&field) => Ok(()),
                Json::Object(_) => Err("a string or a list"),
                Json::List(_) if LIST_ONLY.contains(&field) => Ok(()),
                Json::List(_) => Err("a string"),
                Json::Bool(_) | Json::Number(_) => Err("a string, an object or a list"),
            };
            typed.map_err(|expected| CredentialError::FieldType {
                field: field.to_owned(),
                expected,
            })?;
            fields.insert(field.to_owned(), value.clone());
        }
        Ok(Self { fields })
    }

    // The field's value wrapped at the point a caller uses it, so a key never travels as plain text.
    #[must_use]
    pub fn secret(&self, field: &str) -> Option<SecretString> {
        match self.fields.get(field) {
            Some(Json::Text(value)) => Some(SecretString::from(value.clone())),
            _ => None,
        }
    }
}
