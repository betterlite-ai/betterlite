use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::settings::JsonValue;
use betterlite_config::sections::settings::settings_resolution::{
    EmptyDbValue, FieldDescriptor, FieldSource, resolve_fields,
};
use betterlite_db::stored_cipher::DbCipher;
use betterlite_secrets::secret_codec::{DecryptPolicy, FailureLog, FailureValue, StoredValue};
use secrecy::{ExposeSecret as _, SecretString};
use serde::de::DeserializeOwned;
use thiserror::Error;

use crate::sso_types::records::{RoleMappings, SsoConfig, TeamMappings, UiAccessMode};

mod tests;

pub const SSO_DESCRIPTORS: [FieldDescriptor; 16] = [
    FieldDescriptor {
        field_name: "google_client_id",
        db_key: "google_client_id",
        env_var: "GOOGLE_CLIENT_ID",
        is_secret: false,
        default: None,
    },
    FieldDescriptor {
        field_name: "google_client_secret",
        db_key: "google_client_secret",
        env_var: "GOOGLE_CLIENT_SECRET",
        is_secret: true,
        default: None,
    },
    FieldDescriptor {
        field_name: "microsoft_client_id",
        db_key: "microsoft_client_id",
        env_var: "MICROSOFT_CLIENT_ID",
        is_secret: false,
        default: None,
    },
    FieldDescriptor {
        field_name: "microsoft_client_secret",
        db_key: "microsoft_client_secret",
        env_var: "MICROSOFT_CLIENT_SECRET",
        is_secret: true,
        default: None,
    },
    FieldDescriptor {
        field_name: "microsoft_tenant",
        db_key: "microsoft_tenant",
        env_var: "MICROSOFT_TENANT",
        is_secret: false,
        default: None,
    },
    FieldDescriptor {
        field_name: "generic_client_id",
        db_key: "generic_client_id",
        env_var: "GENERIC_CLIENT_ID",
        is_secret: false,
        default: None,
    },
    FieldDescriptor {
        field_name: "generic_client_secret",
        db_key: "generic_client_secret",
        env_var: "GENERIC_CLIENT_SECRET",
        is_secret: true,
        default: None,
    },
    FieldDescriptor {
        field_name: "generic_authorization_endpoint",
        db_key: "generic_authorization_endpoint",
        env_var: "GENERIC_AUTHORIZATION_ENDPOINT",
        is_secret: false,
        default: None,
    },
    FieldDescriptor {
        field_name: "generic_token_endpoint",
        db_key: "generic_token_endpoint",
        env_var: "GENERIC_TOKEN_ENDPOINT",
        is_secret: false,
        default: None,
    },
    FieldDescriptor {
        field_name: "generic_userinfo_endpoint",
        db_key: "generic_userinfo_endpoint",
        env_var: "GENERIC_USERINFO_ENDPOINT",
        is_secret: false,
        default: None,
    },
    FieldDescriptor {
        field_name: "generic_scope",
        db_key: "generic_scope",
        env_var: "GENERIC_SCOPE",
        is_secret: false,
        default: Some("openid email profile"),
    },
    FieldDescriptor {
        field_name: "saml_idp_metadata_url",
        db_key: "saml_idp_metadata_url",
        env_var: "SAML_IDP_METADATA_URL",
        is_secret: false,
        default: None,
    },
    FieldDescriptor {
        field_name: "saml_idp_metadata_xml",
        db_key: "saml_idp_metadata_xml",
        env_var: "SAML_IDP_METADATA_XML",
        is_secret: false,
        default: None,
    },
    FieldDescriptor {
        field_name: "saml_sp_entity_id",
        db_key: "saml_sp_entity_id",
        env_var: "SAML_SP_ENTITY_ID",
        is_secret: false,
        default: None,
    },
    FieldDescriptor {
        field_name: "saml_allow_unsolicited",
        db_key: "saml_allow_unsolicited",
        env_var: "SAML_ALLOW_UNSOLICITED",
        is_secret: false,
        default: None,
    },
    FieldDescriptor {
        field_name: "proxy_base_url",
        db_key: "proxy_base_url",
        env_var: "PROXY_BASE_URL",
        is_secret: false,
        default: None,
    },
];

// role_mappings and team_mappings are JSON sub-objects on the row, never env-backed scalars
const STRUCTURED_KEYS: [&str; 2] = ["role_mappings", "team_mappings"];

// decrypt_value_helper(return_original_value=True): a failed decryption logs and keeps the stored value
const KEEP_ORIGINAL: DecryptPolicy = DecryptPolicy {
    log: FailureLog::Error,
    value: FailureValue::Original,
};

#[derive(Debug, Clone)]
pub struct ResolvedSsoConfig {
    pub config: SsoConfig,
    pub provenance: BTreeMap<String, FieldSource>,
}

#[derive(Debug, Error)]
pub enum SsoResolutionError {
    #[error("sso_config.{field} is not a valid value: {source}")]
    Field {
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
}

// no const set or map can be built from the const table without a global, so both are computed per call
#[must_use]
pub fn sso_secret_fields() -> BTreeSet<&'static str> {
    SSO_DESCRIPTORS
        .iter()
        .filter(|descriptor| descriptor.is_secret)
        .map(|descriptor| descriptor.field_name)
        .collect()
}

#[must_use]
pub fn sso_field_env_vars() -> BTreeMap<&'static str, &'static str> {
    SSO_DESCRIPTORS
        .iter()
        .map(|descriptor| (descriptor.field_name, descriptor.env_var))
        .collect()
}

pub fn resolve_sso_config(
    sso_db_settings: Option<&BTreeMap<String, JsonValue>>,
    environment: &BTreeMap<String, SecretString>,
    cipher: &DbCipher,
) -> Result<ResolvedSsoConfig, SsoResolutionError> {
    let raw = sso_db_settings.cloned().unwrap_or_default();
    let decrypted = decrypt_row(&raw, cipher);
    let resolved = resolve_fields(
        &SSO_DESCRIPTORS,
        &decrypted,
        environment,
        EmptyDbValue::Absent,
    );
    let mut values = BTreeMap::new();
    let mut provenance = BTreeMap::new();
    for field in resolved {
        provenance.insert(field.field_name.to_owned(), field.source);
        values.insert(field.field_name, field.value);
    }
    let config = build_config(&values, &decrypted, &raw)?;
    Ok(ResolvedSsoConfig { config, provenance })
}

fn decrypt_row(
    raw: &BTreeMap<String, JsonValue>,
    cipher: &DbCipher,
) -> BTreeMap<String, JsonValue> {
    let mut decrypted = BTreeMap::new();
    for (key, value) in raw {
        if STRUCTURED_KEYS.contains(&key.as_str()) {
            continue;
        }
        let JsonValue::String(text) = value else {
            decrypted.insert(key.clone(), value.clone());
            continue;
        };
        let stored = StoredValue::Text(SecretString::from(text.clone()));
        let opened = match cipher.decrypt_value(stored, key, KEEP_ORIGINAL) {
            Some(StoredValue::Text(plaintext)) => {
                JsonValue::String(plaintext.expose_secret().to_owned())
            }
            Some(StoredValue::Other(other)) => other,
            None => JsonValue::Null,
        };
        decrypted.insert(key.clone(), opened);
    }
    decrypted
}

fn build_config(
    values: &BTreeMap<&'static str, Option<SecretString>>,
    decrypted: &BTreeMap<String, JsonValue>,
    raw: &BTreeMap<String, JsonValue>,
) -> Result<SsoConfig, SsoResolutionError> {
    Ok(SsoConfig {
        google_client_id: resolved_text(values, "google_client_id"),
        google_client_secret: values
            .get("google_client_secret")
            .and_then(Option::as_ref)
            .cloned(),
        microsoft_client_id: resolved_text(values, "microsoft_client_id"),
        microsoft_client_secret: values
            .get("microsoft_client_secret")
            .and_then(Option::as_ref)
            .cloned(),
        microsoft_tenant: resolved_text(values, "microsoft_tenant"),
        generic_client_id: resolved_text(values, "generic_client_id"),
        generic_client_secret: values
            .get("generic_client_secret")
            .and_then(Option::as_ref)
            .cloned(),
        generic_authorization_endpoint: resolved_text(values, "generic_authorization_endpoint"),
        generic_token_endpoint: resolved_text(values, "generic_token_endpoint"),
        generic_userinfo_endpoint: resolved_text(values, "generic_userinfo_endpoint"),
        generic_scope: resolved_text(values, "generic_scope"),
        saml_idp_metadata_url: resolved_text(values, "saml_idp_metadata_url"),
        saml_idp_metadata_xml: resolved_text(values, "saml_idp_metadata_xml"),
        saml_sp_entity_id: resolved_text(values, "saml_sp_entity_id"),
        saml_allow_unsolicited: resolved_text(values, "saml_allow_unsolicited"),
        proxy_base_url: resolved_text(values, "proxy_base_url"),
        user_email: optional_typed(decrypted.get("user_email"), "user_email")?,
        ui_access_mode: optional_typed(decrypted.get("ui_access_mode"), "ui_access_mode")?,
        role_mappings: optional_mapping(raw, "role_mappings")?,
        team_mappings: optional_mapping(raw, "team_mappings")?,
    })
}

fn resolved_text(
    values: &BTreeMap<&'static str, Option<SecretString>>,
    name: &str,
) -> Option<String> {
    values
        .get(name)
        .and_then(Option::as_ref)
        .map(|value| value.expose_secret().to_owned())
}

fn optional_typed<T>(
    value: Option<&JsonValue>,
    field: &'static str,
) -> Result<Option<T>, SsoResolutionError>
where
    T: DeserializeOwned,
{
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    serde_json::from_value(value.clone())
        .map(Some)
        .map_err(|source| SsoResolutionError::Field { field, source })
}

fn optional_mapping<T>(
    raw: &BTreeMap<String, JsonValue>,
    field: &'static str,
) -> Result<Option<T>, SsoResolutionError>
where
    T: DeserializeOwned,
{
    match raw.get(field) {
        Some(value @ JsonValue::Object(_)) => serde_json::from_value(value.clone())
            .map(Some)
            .map_err(|source| SsoResolutionError::Field { field, source }),
        None | Some(_) => Ok(None),
    }
}
