use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::resource_rows::model::ProxyModelTable;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::deployment_identity::params::litellm_params;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secrets::secret_codec::{DecryptPolicy, FailureLog, FailureValue, StoredValue};
use secrecy::{ExposeSecret as _, SecretString};

use super::convert::json_of;
use super::error::DbRowDecryptError;
use super::model_info::model_info_for;
use super::warnings::Warnings;

// decrypt_value_helper's default "error" mode with return_original_value=True: the stored value stays.
const KEEP_ORIGINAL: DecryptPolicy = DecryptPolicy {
    log: FailureLog::Error,
    value: FailureValue::Original,
};

// _resolve_db_litellm_param: decrypt a stored string, then read an os.environ reference through the secret lookup.
pub fn resolve_db_param(
    key: &str,
    value: &JsonData,
    cipher: &DbCipher,
    secrets: &SecretLookup,
    environment: &BTreeMap<String, String>,
) -> Result<JsonData, DbRowDecryptError> {
    let JsonData::String(text) = value else {
        return Ok(value.clone());
    };
    let stored = StoredValue::<JsonData>::Text(SecretString::from(text.clone()));
    let Some(StoredValue::Text(opened)) = cipher.decrypt_value(stored, key, KEEP_ORIGINAL) else {
        return Ok(value.clone());
    };
    let plaintext = opened.expose_secret();
    if !plaintext.starts_with("os.environ/") {
        return Ok(JsonData::String(plaintext.to_owned()));
    }
    let resolved = secrets.get_secret_str(environment, plaintext, None)?;
    Ok(resolved.map_or(JsonData::Null, |secret| {
        JsonData::String(secret.expose_secret().to_owned())
    }))
}

// decrypt_model_list_from_db: each row's params resolved and validated, invalid rows skipped, as Deployment.to_json.
pub fn decrypt_model_rows(
    rows: &[ProxyModelTable],
    cipher: &DbCipher,
    secrets: &SecretLookup,
    environment: &BTreeMap<String, String>,
    warnings: &Warnings,
) -> Result<Vec<JsonObject>, DbRowDecryptError> {
    let mut deployments = Vec::with_capacity(rows.len());
    for row in rows {
        let mut resolved = JsonObject::new();
        for (key, value) in &row.fields.litellm_params.0 {
            let decrypted = resolve_db_param(key, value, cipher, secrets, environment)?;
            resolved.insert(key.clone(), json_of(&decrypted));
        }
        let params = match litellm_params(&resolved) {
            Ok(params) => params,
            Err(error) => {
                tracing::error!("Invalid model added to proxy db. Invalid litellm params. {error}");
                continue;
            }
        };
        let deployment = DeploymentSpec::new(
            row.fields.model_name.clone(),
            params,
            model_info_for(row, false, warnings),
            JsonObject::new(),
            || uuid::Uuid::new_v4().to_string(),
        )?;
        deployments.push(deployment.to_json());
    }
    Ok(deployments)
}
