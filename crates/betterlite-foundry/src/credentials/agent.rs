use std::collections::BTreeMap;

use betterlite_azure::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use betterlite_azure::credentials::source::{CallableTokenProvider, CredentialSources, TokenReply};
use betterlite_azure::oidc_exchange::OidcExchangeInput;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_secret_managers::secret_lookup::SecretLookup;
use secrecy::{ExposeSecret as _, SecretString};

use super::error::CredentialsError;
use super::{
    AZURE_AI_AGENTS_SCOPE, AZURE_ENTRA_CREDENTIAL_HELP, ENTRA_PARAM_KEYS, OS_ENVIRON_PREFIX,
};

const OIDC_PREFIX: &str = "oidc/";

pub struct AgentTokenRequest<'data> {
    pub params: &'data Object,
    pub environment: &'data BTreeMap<String, String>,
    pub secrets: &'data SecretLookup,
    pub sources: &'data dyn CredentialSources,
}

#[derive(Debug, Clone, Default)]
struct AgentSecrets {
    azure_ad_token: Option<SecretString>,
    tenant_id: Option<String>,
    client_id: Option<String>,
    client_secret: Option<SecretString>,
    azure_scope: Option<String>,
    azure_username: Option<String>,
    azure_password: Option<SecretString>,
}

impl AgentSecrets {
    fn resolve(request: &AgentTokenRequest<'_>) -> Result<Self, CredentialsError> {
        Ok(Self {
            azure_ad_token: resolve_config_secret(request, request.params.get("azure_ad_token"))?,
            tenant_id: resolve_config_secret(request, request.params.get("tenant_id"))?
                .map(|secret| secret.expose_secret().to_owned()),
            client_id: resolve_config_secret(request, request.params.get("client_id"))?
                .map(|secret| secret.expose_secret().to_owned()),
            client_secret: resolve_config_secret(request, request.params.get("client_secret"))?,
            azure_scope: resolve_config_secret(request, request.params.get("azure_scope"))?
                .map(|secret| secret.expose_secret().to_owned()),
            azure_username: resolve_config_secret(request, request.params.get("azure_username"))?
                .map(|secret| secret.expose_secret().to_owned()),
            azure_password: resolve_config_secret(request, request.params.get("azure_password"))?,
        })
    }

    fn scope(&self) -> String {
        self.azure_scope
            .as_deref()
            .filter(|text| !text.is_empty())
            .unwrap_or(AZURE_AI_AGENTS_SCOPE)
            .to_owned()
    }

    fn client_secret_record(&self, scope: &str) -> Option<ClientSecretRecord> {
        let identity = (
            self.tenant_id.as_deref().filter(|text| !text.is_empty())?,
            self.client_id.as_deref().filter(|text| !text.is_empty())?,
            self.client_secret
                .as_ref()
                .filter(|secret| !secret.expose_secret().is_empty())?,
        );
        let (tenant_id, client_id, client_secret) = identity;
        Some(ClientSecretRecord {
            tenant_id: tenant_id.to_owned(),
            client_id: client_id.to_owned(),
            client_secret: client_secret.clone(),
            scope: scope.to_owned(),
        })
    }

    fn username_password_record(&self, scope: &str) -> Option<UsernamePasswordRecord> {
        let client_id = self.client_id.as_deref().filter(|text| !text.is_empty())?;
        let username = self
            .azure_username
            .as_deref()
            .filter(|text| !text.is_empty())?;
        let password = self
            .azure_password
            .as_ref()
            .filter(|secret| !secret.expose_secret().is_empty())?;
        let record = UsernamePasswordRecord {
            client_id: client_id.to_owned(),
            username: username.to_owned(),
            password: password.clone(),
            tenant_id: None,
            scope: scope.to_owned(),
        };
        Some(record)
    }

    fn configured_token(
        &self,
        request: &AgentTokenRequest<'_>,
        scope: &str,
    ) -> Result<SecretString, CredentialsError> {
        let token = self
            .azure_ad_token
            .as_ref()
            .filter(|secret| !secret.expose_secret().is_empty());
        let federated =
            token.is_some_and(|assertion| assertion.expose_secret().starts_with(OIDC_PREFIX));
        if federated
            && let (Some(azure_ad_token), Some(tenant_id), Some(client_id)) = (
                token,
                self.tenant_id.as_deref().filter(|text| !text.is_empty()),
                self.client_id.as_deref().filter(|text| !text.is_empty()),
            )
        {
            let input = OidcExchangeInput {
                azure_ad_token: azure_ad_token.clone(),
                azure_client_id: Some(client_id.to_owned()),
                azure_tenant_id: Some(tenant_id.to_owned()),
                scope: Some(scope.to_owned()),
            };
            return Ok(request.sources.oidc(&input)?);
        }
        token
            .filter(|_| !federated)
            .cloned()
            .ok_or(CredentialsError::AgentTokenUnresolved {
                help: AZURE_ENTRA_CREDENTIAL_HELP,
            })
    }
}

pub fn get_azure_ai_agent_entra_token(
    request: &AgentTokenRequest<'_>,
) -> Result<SecretString, CredentialsError> {
    let resolved = AgentSecrets::resolve(request)?;
    let scope = resolved.scope();
    if let Some(record) = resolved.client_secret_record(&scope) {
        return invoke(request.sources.client_secret(&record)?.as_ref());
    }
    if let Some(record) = resolved.username_password_record(&scope) {
        return invoke(request.sources.username_password(&record)?.as_ref());
    }
    resolved.configured_token(request, &scope)
}

// the async caller offloads this blocking lookup off the runtime thread
pub fn resolve_agent_auth_header(
    request: &AgentTokenRequest<'_>,
) -> Result<BTreeMap<String, String>, CredentialsError> {
    let token = get_azure_ai_agent_entra_token(request)?;
    Ok(BTreeMap::from([(
        "Authorization".to_owned(),
        format!("Bearer {}", token.expose_secret()),
    )]))
}

#[must_use]
pub fn has_azure_entra_params(params: Option<&Object>) -> bool {
    params.is_some_and(|values| {
        ENTRA_PARAM_KEYS
            .iter()
            .any(|key| values.get(*key).is_some_and(Json::is_truthy))
    })
}

fn resolve_config_secret(
    request: &AgentTokenRequest<'_>,
    value: Option<&Json>,
) -> Result<Option<SecretString>, CredentialsError> {
    let Some(Json::Text(text)) = value else {
        return Ok(None);
    };
    if text.is_empty() {
        return Ok(None);
    }
    if !text.starts_with(OS_ENVIRON_PREFIX) {
        return Ok(Some(SecretString::from(text.to_owned())));
    }
    Ok(request
        .secrets
        .get_secret_str(request.environment, text, None)?)
}

fn invoke(provider: &dyn CallableTokenProvider) -> Result<SecretString, CredentialsError> {
    match provider.token() {
        Ok(TokenReply::Text(token)) => Ok(token),
        Ok(TokenReply::NonText { type_name }) => {
            Err(CredentialsError::TokenNotString { type_name })
        }
        Err(failure) => Err(CredentialsError::ProviderFailed {
            message: failure.message,
        }),
    }
}
