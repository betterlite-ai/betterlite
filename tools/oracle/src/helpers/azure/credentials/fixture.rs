use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_azure::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use betterlite_azure::credentials::source::{
    CallableTokenProvider, CredentialSources, ProviderFailure, SourceFailure, TokenReply,
};
use betterlite_azure::oidc_exchange::OidcExchangeInput;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ProviderScript {
    Token { token: String },
    NonText { type_name: String },
    Raise { class_name: String, message: String },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum SourceScript {
    Token { token: String },
    Raise { class_name: String, message: String },
}

#[derive(Debug, Clone)]
pub(super) enum Outcome {
    Token(String),
    NonText(String),
    Raise { class: String, message: String },
}

// the Python fixtures raise the named class and fall back to RuntimeError for any other name
fn class_name(name: &str) -> String {
    match name {
        "ValueError" | "KeyError" | "TypeError" => name.to_owned(),
        _ => "RuntimeError".to_owned(),
    }
}

impl ProviderScript {
    pub(super) fn outcome(&self) -> Outcome {
        match self {
            Self::Token { token } => Outcome::Token(token.clone()),
            Self::NonText { type_name } => Outcome::NonText(type_name.clone()),
            Self::Raise {
                class_name: name,
                message,
            } => Outcome::Raise {
                class: class_name(name),
                message: message.clone(),
            },
        }
    }
}

pub(super) struct ScriptedProvider {
    outcome: Outcome,
    calls: Arc<Mutex<usize>>,
}

impl CallableTokenProvider for ScriptedProvider {
    fn token(&self) -> Result<TokenReply, ProviderFailure> {
        let mut count = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        *count = count.saturating_add(1);
        drop(count);
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

pub(super) fn scripted(
    outcome: Outcome,
    calls: &Arc<Mutex<usize>>,
) -> Arc<dyn CallableTokenProvider> {
    Arc::new(ScriptedProvider {
        outcome,
        calls: Arc::clone(calls),
    })
}

pub(super) struct FixtureSources {
    scripts: BTreeMap<String, SourceScript>,
    calls: Mutex<Vec<Value>>,
    pub(super) provider_calls: Arc<Mutex<usize>>,
}

impl FixtureSources {
    pub(super) fn new(scripts: &BTreeMap<String, SourceScript>) -> Self {
        Self {
            scripts: scripts.clone(),
            calls: Mutex::new(Vec::new()),
            provider_calls: Arc::new(Mutex::new(0)),
        }
    }

    pub(super) fn calls(&self) -> Value {
        Value::Array(
            self.calls
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        )
    }

    fn record(&self, call: Value) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
    }

    fn provider(
        &self,
        script: Option<&SourceScript>,
        default_token: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + '_>, SourceFailure> {
        match script {
            None => Ok(scripted(
                Outcome::Token(default_token.to_owned()),
                &self.provider_calls,
            )),
            Some(SourceScript::Token { token }) => Ok(scripted(
                Outcome::Token(token.clone()),
                &self.provider_calls,
            )),
            Some(SourceScript::Raise {
                class_name: name,
                message,
            }) => Err(SourceFailure {
                class: class_name(name),
                status: None,
                message: message.clone(),
            }),
        }
    }
}

impl CredentialSources for FixtureSources {
    fn client_secret<'source>(
        &'source self,
        record: &ClientSecretRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        self.record(json!({"source": "client_secret", "tenant_id": record.tenant_id, "client_id": record.client_id, "client_secret": record.client_secret.expose_secret(), "scope": record.scope}));
        self.provider(self.scripts.get("client_secret"), "client-secret-token")
    }

    fn username_password<'source>(
        &'source self,
        record: &UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        self.record(json!({"source": "username_password", "client_id": record.client_id, "username": record.username, "password": record.password.expose_secret(), "scope": record.scope}));
        self.provider(self.scripts.get("username_password"), "username-token")
    }

    fn sdk_refresh<'source>(
        &'source self,
        scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        self.record(json!({"source": "sdk_refresh", "scope": scope}));
        self.provider(self.scripts.get("sdk_refresh"), "refresh-token")
    }

    fn default_credential<'source>(
        &'source self,
        scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>> {
        self.record(json!({"source": "default_credential", "scope": scope}));
        match self.scripts.get("default_credential") {
            None => Some(scripted(
                Outcome::Token("default-token".to_owned()),
                &self.provider_calls,
            )),
            Some(SourceScript::Token { token }) => Some(scripted(
                Outcome::Token(token.clone()),
                &self.provider_calls,
            )),
            Some(SourceScript::Raise { .. }) => None,
        }
    }

    fn oidc(&self, input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        self.record(json!({
            "source": "oidc",
            "token": input.azure_ad_token.expose_secret(),
            "client_id": input.azure_client_id,
            "tenant_id": input.azure_tenant_id,
            "scope": input.scope,
        }));
        match self.scripts.get("oidc") {
            None => Ok(SecretString::from("oidc-token")),
            Some(SourceScript::Token { token }) => Ok(SecretString::from(token.clone())),
            Some(SourceScript::Raise {
                class_name: name,
                message,
            }) => Err(SourceFailure {
                class: class_name(name),
                status: None,
                message: message.clone(),
            }),
        }
    }
}
