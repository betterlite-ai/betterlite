use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
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
pub(crate) enum Script {
    Token { token: String },
    NonText { type_name: String },
    Raise { class_name: String, message: String },
}

// the Python fixtures raise the named class and fall back to RuntimeError for any other name
fn class_of(name: &str) -> String {
    match name {
        "ValueError" | "KeyError" | "TypeError" => name.to_owned(),
        _ => "RuntimeError".to_owned(),
    }
}

pub(super) struct ScriptedProvider {
    script: Script,
    calls: Arc<AtomicUsize>,
}

impl CallableTokenProvider for ScriptedProvider {
    fn token(&self) -> Result<TokenReply, ProviderFailure> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        match &self.script {
            Script::Token { token } => Ok(TokenReply::Text(SecretString::from(token.clone()))),
            Script::NonText { type_name } => Ok(TokenReply::NonText {
                type_name: type_name.clone(),
            }),
            Script::Raise {
                class_name,
                message,
            } => Err(ProviderFailure {
                class: class_of(class_name),
                message: message.clone(),
            }),
        }
    }
}

pub(super) fn scripted(script: Script, calls: &Arc<AtomicUsize>) -> Arc<dyn CallableTokenProvider> {
    Arc::new(ScriptedProvider {
        script,
        calls: Arc::clone(calls),
    })
}

pub(super) struct FixtureSources {
    scripts: BTreeMap<String, Script>,
    calls: Mutex<Vec<Value>>,
    pub(super) provider_calls: Arc<AtomicUsize>,
}

impl FixtureSources {
    pub(super) fn new(scripts: &BTreeMap<String, Script>) -> Self {
        Self {
            scripts: scripts.clone(),
            calls: Mutex::new(Vec::new()),
            provider_calls: Arc::new(AtomicUsize::new(0)),
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

    fn provider(
        &self,
        name: &str,
        call: Value,
        default_token: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + '_>, SourceFailure> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
        let script = self
            .scripts
            .get(name)
            .cloned()
            .unwrap_or_else(|| Script::Token {
                token: default_token.to_owned(),
            });
        if let Script::Raise {
            class_name,
            message,
        } = &script
        {
            return Err(SourceFailure {
                class: class_of(class_name),
                status: None,
                message: message.clone(),
            });
        }
        Ok(scripted(script, &self.provider_calls))
    }
}

impl CredentialSources for FixtureSources {
    fn client_secret<'source>(
        &'source self,
        record: &ClientSecretRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        let call = json!({"source": "client_secret", "tenant_id": record.tenant_id, "client_id": record.client_id, "client_secret": record.client_secret.expose_secret(), "scope": record.scope});
        self.provider("client_secret", call, "client-secret-token")
    }

    fn username_password<'source>(
        &'source self,
        record: &UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        let call = json!({"source": "username_password", "client_id": record.client_id, "azure_username": record.username, "azure_password": record.password.expose_secret(), "scope": record.scope});
        self.provider("username_password", call, "username-token")
    }

    fn sdk_refresh<'source>(
        &'source self,
        scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        let call = json!({"source": "sdk_refresh", "scope": scope});
        self.provider("sdk_refresh", call, "refresh-token")
    }

    fn default_credential<'source>(
        &'source self,
        scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>> {
        let call = json!({"source": "default_credential", "scope": scope});
        self.provider("default_credential", call, "default-token")
            .ok()
    }

    fn oidc(&self, input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        let call = json!({
            "source": "oidc",
            "azure_ad_token": input.azure_ad_token.expose_secret(),
            "azure_client_id": input.azure_client_id,
            "azure_tenant_id": input.azure_tenant_id,
            "scope": input.scope,
        });
        let reply = self.provider("oidc", call, "oidc-token")?.token();
        match reply {
            Ok(TokenReply::Text(token)) => Ok(token),
            Ok(TokenReply::NonText { type_name }) => Err(SourceFailure {
                class: "TypeError".to_owned(),
                status: None,
                message: type_name,
            }),
            Err(failure) => Err(SourceFailure {
                class: failure.class,
                status: None,
                message: failure.message,
            }),
        }
    }
}
