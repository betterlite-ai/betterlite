use std::collections::BTreeMap;

use betterlite_db::managed_resource_rows::mcp::McpEnvVar;
use betterlite_db::mcp_server_encoding::env_vars::decrypt_global_env_var_values;
use betterlite_db::mcp_stored_contracts::secret::StoredSecret;
use betterlite_db::mcp_stored_contracts::{JsonValue, MCPCredentials, MCPTokenEndpointAuthMethod};
use secrecy::{ExposeSecret as _, SecretString};

use super::{BuildError, Decryptor, ServerBuilder};
use crate::server_oauth::merge::{ScopesInput, extract_scopes};
use crate::server_record::SecretCredential;

#[derive(Debug, Default)]
pub struct AwsCredentials {
    pub access_key_id: Option<SecretCredential>,
    pub secret_access_key: Option<SecretCredential>,
    pub session_token: Option<SecretCredential>,
    pub region_name: Option<String>,
    pub service_name: Option<String>,
    pub role_name: Option<String>,
    pub session_name: Option<String>,
}

#[derive(Debug, Default)]
pub(super) struct ResolvedCredentials {
    pub authentication_token: Option<SecretCredential>,
    pub client_id: Option<String>,
    pub client_secret: Option<SecretCredential>,
    pub scopes: Option<Vec<String>>,
    pub configured_scopes: Option<Vec<String>>,
    pub token_endpoint_auth_method: Option<MCPTokenEndpointAuthMethod>,
    pub token_exchange_endpoint: Option<String>,
    pub audience: Option<String>,
    pub subject_token_type: Option<String>,
    pub upstream_resource: Option<String>,
    pub upstream_token_header: Option<String>,
    pub id_jag_resource_token_endpoint: Option<String>,
    pub id_jag_resource: Option<String>,
    pub client_private_key: Option<SecretCredential>,
    pub client_private_key_id: Option<String>,
    pub client_assertion_signing_alg: Option<String>,
    pub token_exchange_profile: Option<String>,
    pub aws: AwsCredentials,
}

impl<D, F, S, R> ServerBuilder<'_, D, F, S, R>
where
    D: Decryptor,
{
    #[must_use]
    pub fn decrypt_credential_field(
        &self,
        value: Option<&str>,
        key: &str,
        encrypted: bool,
    ) -> Option<String> {
        let text = value.filter(|candidate| !candidate.is_empty())?;
        if encrypted {
            return Some(self.decryptor.decrypt(text, key));
        }
        Some(text.to_owned())
    }

    #[must_use]
    pub fn extract_aws_credentials(
        &self,
        credentials: Option<&MCPCredentials>,
        encrypted: bool,
    ) -> AwsCredentials {
        let Some(credentials) = credentials else {
            return AwsCredentials::default();
        };
        AwsCredentials {
            access_key_id: self.credential(
                &credentials.aws_access_key_id,
                "aws_access_key_id",
                encrypted,
            ),
            secret_access_key: self.credential(
                &credentials.aws_secret_access_key,
                "aws_secret_access_key",
                encrypted,
            ),
            session_token: self.credential(
                &credentials.aws_session_token,
                "aws_session_token",
                encrypted,
            ),
            region_name: plain_credential_value(&credentials.aws_region_name),
            service_name: plain_credential_value(&credentials.aws_service_name),
            role_name: plain_credential_value(&credentials.aws_role_name),
            session_name: plain_credential_value(&credentials.aws_session_name),
        }
    }

    pub fn resolve_env_vars_list(
        &self,
        env_vars: Option<&[McpEnvVar]>,
        encrypted: bool,
    ) -> Result<Option<Vec<BTreeMap<String, JsonValue>>>, BuildError> {
        let Some(entries) = env_vars else {
            return Ok(None);
        };
        let mut decrypted = entries.to_vec();
        if encrypted {
            decrypt_global_env_var_values(&mut decrypted, self.env_cipher);
        }
        let mut resolved = Vec::with_capacity(decrypted.len());
        for entry in &decrypted {
            let encoded = serde_json::to_string(entry)?;
            resolved.push(serde_json::from_str(&encoded)?);
        }
        Ok((!resolved.is_empty()).then_some(resolved))
    }

    fn credential(
        &self,
        stored: &Option<Option<StoredSecret>>,
        key: &str,
        encrypted: bool,
    ) -> Option<SecretCredential> {
        let secret = stored.as_ref()?.as_ref()?;
        let text =
            self.decrypt_credential_field(Some(secret.0.expose_secret().as_str()), key, encrypted)?;
        Some(SecretCredential(StoredSecret(SecretString::from(text))))
    }
}

pub(super) fn resolved_credentials<D, F, S, R>(
    builder: &ServerBuilder<'_, D, F, S, R>,
    credentials: Option<&MCPCredentials>,
    encrypted: bool,
) -> ResolvedCredentials
where
    D: Decryptor,
{
    let Some(credentials) = credentials else {
        return ResolvedCredentials::default();
    };
    let mut resolved = ResolvedCredentials::default();
    apply_secret_fields(builder, &mut resolved, credentials, encrypted);
    apply_scope_fields(&mut resolved, credentials);
    apply_token_fields(&mut resolved, credentials);
    resolved.aws = builder.extract_aws_credentials(Some(credentials), encrypted);
    resolved
}

fn apply_secret_fields<D, F, S, R>(
    builder: &ServerBuilder<'_, D, F, S, R>,
    resolved: &mut ResolvedCredentials,
    credentials: &MCPCredentials,
    encrypted: bool,
) where
    D: Decryptor,
{
    resolved.authentication_token =
        builder.credential(&credentials.auth_value, "auth_value", encrypted);
    resolved.client_id = builder.decrypt_credential_field(
        plain_credential_value(&credentials.client_id).as_deref(),
        "client_id",
        encrypted,
    );
    resolved.client_secret =
        builder.credential(&credentials.client_secret, "client_secret", encrypted);
    resolved.client_private_key = builder
        .decrypt_credential_field(
            credentials
                .client_private_key
                .as_ref()
                .and_then(Option::as_ref)
                .map(|secret| secret.0.expose_secret().as_str()),
            "client_private_key",
            encrypted,
        )
        .map(|text| SecretCredential(StoredSecret(SecretString::from(text))));
}

fn apply_scope_fields(resolved: &mut ResolvedCredentials, credentials: &MCPCredentials) {
    let stored_scopes = credentials.scopes.as_ref().and_then(Option::as_ref);
    let scope_values: Option<Vec<JsonValue>> =
        stored_scopes.map(|values| values.iter().cloned().map(JsonValue::String).collect());
    resolved.scopes = scope_values
        .as_deref()
        .and_then(|values| extract_scopes(ScopesInput::List(values)));
    resolved.configured_scopes = stored_scopes
        .filter(|values| !values.is_empty() && values.iter().all(|value| !value.is_empty()))
        .cloned();
    resolved.token_endpoint_auth_method = credentials.token_endpoint_auth_method.flatten();
}

fn apply_token_fields(resolved: &mut ResolvedCredentials, credentials: &MCPCredentials) {
    resolved.token_exchange_endpoint = plain_credential_value(&credentials.token_exchange_endpoint);
    resolved.audience = plain_credential_value(&credentials.audience);
    resolved.subject_token_type = plain_credential_value(&credentials.subject_token_type);
    resolved.upstream_resource = plain_credential_value(&credentials.upstream_resource);
    resolved.upstream_token_header = plain_credential_value(&credentials.upstream_token_header);
    resolved.id_jag_resource_token_endpoint =
        plain_credential_value(&credentials.id_jag_resource_token_endpoint);
    resolved.id_jag_resource = plain_credential_value(&credentials.id_jag_resource);
    resolved.client_private_key_id = plain_credential_value(&credentials.client_private_key_id);
    resolved.client_assertion_signing_alg =
        plain_credential_value(&credentials.client_assertion_signing_alg);
    resolved.token_exchange_profile = plain_credential_value(&credentials.token_exchange_profile);
}

fn plain_credential_value(stored: &Option<Option<String>>) -> Option<String> {
    match stored {
        Some(Some(value)) => Some(value.clone()),
        Some(None) | None => None,
    }
}
