use std::convert::Infallible;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_secrets::error::SecretError;
use betterlite_secrets::secret_codec::StoredValue;
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

use crate::stored_cipher::DbCipher;

// LiteLLM's LITELLM_PROXY_ADMIN_NAME: the audit value for a caller whose user_id is null.
pub const DEFAULT_USER_ID: &str = "default_user_id";

const LITELLM_PARAMS: &str = "litellm_params";
const MODEL_INFO: &str = "model_info";
const MODEL_NAME: &str = "model_name";
const MODEL_ID: &str = "id";

// What _add_model_to_db builds before it inserts: both JSON texts sealed, the audit fields chosen, the insert left to the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedModel {
    pub model_id: String,
    pub model_name: String,
    pub litellm_params: String,
    pub model_info: String,
    pub created_by: String,
    pub updated_by: String,
}

#[derive(Debug, Error)]
pub enum ModelSecretsError {
    #[error("the model record has no {field} object")]
    MissingObject { field: &'static str },
    #[error("the model record has no {field} text")]
    MissingText { field: &'static str },
    #[error("the model record's {field} does not serialize")]
    Serialize {
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("encrypting the model's {key} parameter failed")]
    Encryption {
        key: String,
        #[source]
        source: SecretError,
    },
}

// _add_model_to_db's record prep: encrypt_value_helper on every non-null litellm_params value, model_dump_json for both nested models.
pub fn prepare_encrypted_model(
    deployment: &JsonObject,
    cipher: &DbCipher,
    new_master_key: &SecretString,
    actor: Option<&str>,
) -> Result<PreparedModel, ModelSecretsError> {
    let params = object_field(deployment, LITELLM_PARAMS)?;
    let info = object_field(deployment, MODEL_INFO)?;
    let sealed = seal_params(params, cipher, new_master_key)?;
    let params_json =
        serde_json::to_string(&sealed).map_err(|source| ModelSecretsError::Serialize {
            field: LITELLM_PARAMS,
            source,
        })?;
    let info_json = serde_json::to_string(&without_nulls(info)).map_err(|source| {
        ModelSecretsError::Serialize {
            field: MODEL_INFO,
            source,
        }
    })?;
    let Some(Json::Text(model_id)) = info.get(MODEL_ID) else {
        return Err(ModelSecretsError::MissingText { field: MODEL_ID });
    };
    let Some(Json::Text(model_name)) = deployment.get(MODEL_NAME) else {
        return Err(ModelSecretsError::MissingText { field: MODEL_NAME });
    };
    let audit = actor.unwrap_or(DEFAULT_USER_ID).to_owned();
    Ok(PreparedModel {
        model_id: model_id.to_owned(),
        model_name: model_name.to_owned(),
        litellm_params: params_json,
        model_info: info_json,
        created_by: audit.clone(),
        updated_by: audit,
    })
}

fn object_field<'json>(
    deployment: &'json JsonObject,
    field: &'static str,
) -> Result<&'json JsonObject, ModelSecretsError> {
    let Some(Json::Object(object)) = deployment.get(field) else {
        return Err(ModelSecretsError::MissingObject { field });
    };
    Ok(object)
}

// exclude_none: a null parameter is left out of the stored record rather than sealed, and a string is sealed with the explicit key.
fn seal_params(
    params: &JsonObject,
    cipher: &DbCipher,
    new_master_key: &SecretString,
) -> Result<JsonObject, ModelSecretsError> {
    let mut sealed = JsonObject::new();
    for (name, value) in params {
        let sealed_value = match value {
            Json::Null => continue,
            Json::Text(text) => sealed_text(name, text, cipher, new_master_key)?,
            other @ (Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
                other.clone()
            }
        };
        sealed.insert(name.clone(), sealed_value);
    }
    Ok(sealed)
}

fn sealed_text(
    name: &str,
    text: &str,
    cipher: &DbCipher,
    new_master_key: &SecretString,
) -> Result<Json, ModelSecretsError> {
    let stored = StoredValue::<Infallible>::Text(SecretString::from(text.to_owned()));
    let sealed = cipher
        .encrypt_value(stored, Some(new_master_key))
        .map_err(|source| ModelSecretsError::Encryption {
            key: name.to_owned(),
            source,
        })?;
    match sealed {
        StoredValue::Text(ciphertext) => Ok(Json::Text(ciphertext.expose_secret().to_owned())),
        StoredValue::Other(never) => match never {},
    }
}

fn without_nulls(info: &JsonObject) -> JsonObject {
    info.iter()
        .filter(|(_, value)| !matches!(value, Json::Null))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

mod tests;
