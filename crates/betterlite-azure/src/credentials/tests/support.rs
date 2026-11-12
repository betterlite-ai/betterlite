use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::sections::azure_credentials::{DEFAULT_SCOPE, DeploymentSettings};
use secrecy::{ExposeSecret as _, SecretString};

use crate::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use crate::credentials::selection::AzureCredentialRequest;
use crate::credentials::source::{
    CallableTokenProvider, CredentialSources, ProviderFailure, SourceFailure, TokenReply,
};
use crate::oidc_exchange::OidcExchangeInput;

#[derive(Debug, Clone)]
pub(super) enum Outcome {
    Token(String),
    NonText(String),
    Raise { class: String, message: String },
}

pub(super) struct ScriptedProvider {
    pub(super) outcome: Outcome,
    pub(super) calls: Arc<Mutex<usize>>,
}

impl CallableTokenProvider for ScriptedProvider {
    fn token(&self) -> Result<TokenReply, ProviderFailure> {
        let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        *calls = calls.saturating_add(1);
        drop(calls);
        match &self.outcome {
            Outcome::Token(text) => Ok(TokenReply::Text(SecretString::from(text.clone()))),
            Outcome::NonText(type_name) => Ok(TokenReply::NonText {
                type_name: type_name.clone(),
            }),
            Outcome::Raise { class, message } => Err(ProviderFailure {
                class: class.clone(),
                message: message.clone(),
            }),
        }
    }
}

pub(super) struct FixtureSources {
    pub(super) client_secret: Result<Outcome, SourceFailure>,
    pub(super) username_password: Result<Outcome, SourceFailure>,
    pub(super) sdk_refresh: Result<Outcome, SourceFailure>,
    pub(super) default_credential: Option<Outcome>,
    pub(super) oidc: Result<String, SourceFailure>,
    calls: Mutex<Vec<String>>,
    pub(super) provider_calls: Arc<Mutex<usize>>,
}

impl FixtureSources {
    pub(super) fn new() -> Self {
        Self {
            client_secret: Ok(Outcome::Token("client-secret-token".to_owned())),
            username_password: Ok(Outcome::Token("username-token".to_owned())),
            sdk_refresh: Ok(Outcome::Token("refresh-token".to_owned())),
            default_credential: Some(Outcome::Token("default-token".to_owned())),
            oidc: Ok("oidc-token".to_owned()),
            calls: Mutex::new(Vec::new()),
            provider_calls: Arc::new(Mutex::new(0)),
        }
    }

    pub(super) fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn provider_calls(&self) -> usize {
        *self
            .provider_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn record(&self, call: String) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
    }

    fn provider(
        &self,
        call: String,
        outcome: &Result<Outcome, SourceFailure>,
    ) -> Result<Arc<dyn CallableTokenProvider + '_>, SourceFailure> {
        self.record(call);
        match outcome {
            Ok(script) => Ok(Arc::new(ScriptedProvider {
                outcome: script.clone(),
                calls: Arc::clone(&self.provider_calls),
            })),
            Err(failure) => Err(failure.clone()),
        }
    }
}

impl CredentialSources for FixtureSources {
    fn client_secret<'source>(
        &'source self,
        record: &ClientSecretRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        self.provider(client_call(record), &self.client_secret)
    }

    fn username_password<'source>(
        &'source self,
        record: &UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        self.provider(username_call(record), &self.username_password)
    }

    fn sdk_refresh<'source>(
        &'source self,
        scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        self.provider(format!("sdk_refresh:{scope}"), &self.sdk_refresh)
    }

    fn default_credential<'source>(
        &'source self,
        scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>> {
        self.record(format!("default_credential:{scope}"));
        self.default_credential.as_ref().map(
            |outcome| -> Arc<dyn CallableTokenProvider + 'source> {
                Arc::new(ScriptedProvider {
                    outcome: outcome.clone(),
                    calls: Arc::clone(&self.provider_calls),
                })
            },
        )
    }

    fn oidc(&self, input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        self.record(format!(
            "oidc:{}:{}:{}:{}",
            input.azure_ad_token.expose_secret(),
            input.azure_client_id.as_deref().unwrap_or("none"),
            input.azure_tenant_id.as_deref().unwrap_or("none"),
            input.scope.as_deref().unwrap_or("none")
        ));
        self.oidc.clone().map(SecretString::from)
    }
}

fn client_call(record: &ClientSecretRecord) -> String {
    let ClientSecretRecord {
        tenant_id,
        client_id,
        client_secret,
        scope,
    } = record;
    format!(
        "client_secret:{tenant_id}:{client_id}:{}:{scope}",
        client_secret.expose_secret()
    )
}

fn username_call(record: &UsernamePasswordRecord) -> String {
    let UsernamePasswordRecord {
        client_id,
        username,
        password,
        tenant_id,
        scope,
    } = record;
    format!(
        "username_password:{client_id}:{username}:{}:{tenant_id:?}:{scope}",
        password.expose_secret()
    )
}

pub(super) fn params() -> DeploymentSettings {
    DeploymentSettings {
        azure_ad_token: None,
        tenant_id: None,
        client_id: None,
        client_secret: None,
        azure_scope: DEFAULT_SCOPE.to_owned(),
        azure_username: None,
        azure_password: None,
    }
}

pub(super) fn request<'data>(
    params: &'data DeploymentSettings,
    refresh_enabled: bool,
    provider: Option<Arc<dyn CallableTokenProvider + 'data>>,
    sources: &'data FixtureSources,
) -> AzureCredentialRequest<'data> {
    AzureCredentialRequest {
        params,
        refresh_enabled,
        provider,
        sources,
    }
}

pub(super) fn scripted(
    outcome: Outcome,
    calls: &Arc<Mutex<usize>>,
) -> Arc<dyn CallableTokenProvider> {
    Arc::new(ScriptedProvider {
        outcome,
        calls: Arc::clone(calls),
    })
}

pub(super) fn pairs(values: &[(&str, &str)]) -> Vec<(String, String)> {
    values
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn map(values: &[(&str, &str)]) -> BTreeMap<String, String> {
    values
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}
