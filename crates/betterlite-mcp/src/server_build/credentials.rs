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
        let Some(stored) = credentials else {
            return AwsCredentials::default();
        };
        AwsCredentials {
            access_key_id: self.credential(
                stored.aws_access_key_id.as_ref().and_then(Option::as_ref),
                "aws_access_key_id",
                encrypted,
            ),
            secret_access_key: self.credential(
                stored
                    .aws_secret_access_key
                    .as_ref()
                    .and_then(Option::as_ref),
                "aws_secret_access_key",
                encrypted,
            ),
            session_token: self.credential(
                stored.aws_session_token.as_ref().and_then(Option::as_ref),
                "aws_session_token",
                encrypted,
            ),
            region_name: stored.aws_region_name.clone().flatten(),
            service_name: stored.aws_service_name.clone().flatten(),
            role_name: stored.aws_role_name.clone().flatten(),
            session_name: stored.aws_session_name.clone().flatten(),
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
        stored: Option<&StoredSecret>,
        key: &str,
        encrypted: bool,
    ) -> Option<SecretCredential> {
        let secret = stored?;
        let text = self.decrypt_credential_field(Some(secret.0.expose_secret()), key, encrypted)?;
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
    let Some(stored) = credentials else {
        return ResolvedCredentials::default();
    };
    let mut resolved = ResolvedCredentials::default();
    apply_secret_fields(builder, &mut resolved, stored, encrypted);
    apply_scope_fields(&mut resolved, stored);
    apply_token_fields(&mut resolved, stored);
    resolved.aws = builder.extract_aws_credentials(Some(stored), encrypted);
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
    resolved.authentication_token = builder.credential(
        credentials.auth_value.as_ref().and_then(Option::as_ref),
        "auth_value",
        encrypted,
    );
    resolved.client_id = builder.decrypt_credential_field(
        credentials
            .client_id
            .as_ref()
            .and_then(Option::as_ref)
            .map(String::as_str),
        "client_id",
        encrypted,
    );
    resolved.client_secret = builder.credential(
        credentials.client_secret.as_ref().and_then(Option::as_ref),
        "client_secret",
        encrypted,
    );
    resolved.client_private_key = builder
        .decrypt_credential_field(
            credentials
                .client_private_key
                .as_ref()
                .and_then(Option::as_ref)
                .map(|secret| secret.0.expose_secret()),
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
    resolved.token_exchange_endpoint = credentials.token_exchange_endpoint.clone().flatten();
    resolved.audience = credentials.audience.clone().flatten();
    resolved.subject_token_type = credentials.subject_token_type.clone().flatten();
    resolved.upstream_resource = credentials.upstream_resource.clone().flatten();
    resolved.upstream_token_header = credentials.upstream_token_header.clone().flatten();
    resolved.id_jag_resource_token_endpoint =
        credentials.id_jag_resource_token_endpoint.clone().flatten();
    resolved.id_jag_resource = credentials.id_jag_resource.clone().flatten();
    resolved.client_private_key_id = credentials.client_private_key_id.clone().flatten();
    resolved.client_assertion_signing_alg =
        credentials.client_assertion_signing_alg.clone().flatten();
    resolved.token_exchange_profile = credentials.token_exchange_profile.clone().flatten();
}
