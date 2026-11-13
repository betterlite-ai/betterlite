use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;

use betterlite_azure::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use betterlite_azure::credentials::source::{
    CallableTokenProvider, CredentialSources, ProviderFailure, SourceFailure, TokenReply,
};
use betterlite_azure::endpoints::get_client::ClientCache;
use betterlite_azure::endpoints::projection::{CallableProvider, ClientIdentity, ClientKind};
use betterlite_azure::oidc_exchange::OidcExchangeInput;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

use super::CachedChoice;

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Script {
    Token { token: String },
    Raise { class_name: String, message: String },
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SourceScripts {
    #[serde(default)]
    pub(crate) client_secret: Option<Script>,
    #[serde(default)]
    pub(crate) username_password: Option<Script>,
    #[serde(default)]
    pub(crate) refresh: Option<Script>,
    #[serde(default)]
    pub(crate) oidc: Option<Script>,
}

pub(crate) struct Provider {
    script: Script,
}

impl CallableTokenProvider for Provider {
    fn token(&self) -> Result<TokenReply, ProviderFailure> {
        match &self.script {
            Script::Token { token } => Ok(TokenReply::Text(SecretString::from(token.clone()))),
            Script::Raise {
                class_name,
                message,
            } => Err(ProviderFailure {
                class: class_name.clone(),
                message: message.clone(),
            }),
        }
    }
}

pub(crate) fn callable(script: Script, identity: u64) -> CallableProvider {
    let provider: Arc<dyn CallableTokenProvider> = Arc::new(Provider { script });
    CallableProvider { provider, identity }
}

pub(crate) struct Sources {
    scripts: SourceScripts,
    calls: Mutex<Vec<Value>>,
}

impl Sources {
    pub(crate) fn new(scripts: &SourceScripts) -> Self {
        Self {
            scripts: scripts.clone(),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn calls(&self) -> Vec<Value> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, call: Value) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
    }
}

fn token_script(script: Option<&Script>, default_token: &str) -> Script {
    script.cloned().unwrap_or_else(|| Script::Token {
        token: default_token.to_owned(),
    })
}

fn scripted(script: Script) -> Result<Arc<dyn CallableTokenProvider + 'static>, SourceFailure> {
    match script {
        Script::Token { token } => {
            let provider: Arc<dyn CallableTokenProvider> = Arc::new(Provider {
                script: Script::Token { token },
            });
            Ok(provider)
        }
        Script::Raise {
            class_name,
            message,
        } => Err(SourceFailure {
            class: class_name,
            status: None,
            message,
        }),
    }
}

impl CredentialSources for Sources {
    fn client_secret(
        &self,
        record: &ClientSecretRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + '_>, SourceFailure> {
        self.record(json!({
            "source": "client_secret",
            "tenant_id": record.tenant_id,
            "client_id": record.client_id,
            "client_secret": record.client_secret.expose_secret(),
            "scope": record.scope,
        }));
        let script = token_script(self.scripts.client_secret.as_ref(), "client-secret-token");
        scripted(script)
    }

    fn username_password(
        &self,
        record: &UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + '_>, SourceFailure> {
        self.record(json!({
            "source": "username_password",
            "client_id": record.client_id,
            "azure_username": record.username,
            "azure_password": record.password.expose_secret(),
            "scope": record.scope,
        }));
        let script = token_script(self.scripts.username_password.as_ref(), "username-token");
        scripted(script)
    }

    fn sdk_refresh(
        &self,
        scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + '_>, SourceFailure> {
        self.record(json!({"source": "refresh", "scope": scope}));
        let script = token_script(self.scripts.refresh.as_ref(), "refresh-token");
        scripted(script)
    }

    fn default_credential(&self, _scope: &str) -> Option<Arc<dyn CallableTokenProvider + '_>> {
        None
    }

    fn oidc(&self, input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        self.record(json!({
            "source": "oidc",
            "azure_ad_token": input.azure_ad_token.expose_secret(),
            "azure_client_id": input.azure_client_id,
            "azure_tenant_id": input.azure_tenant_id,
            "scope": input.scope,
        }));
        match token_script(self.scripts.oidc.as_ref(), "oidc-token") {
            Script::Token { token } => Ok(SecretString::from(token)),
            Script::Raise {
                class_name,
                message,
            } => Err(SourceFailure {
                class: class_name,
                status: None,
                message,
            }),
        }
    }
}

pub(crate) struct Cache {
    pub(crate) preloaded: Option<ClientKind>,
    pub(crate) sets: Vec<(ClientIdentity, ClientKind, bool)>,
}

impl Cache {
    pub(crate) const fn for_choice(choice: CachedChoice) -> Self {
        Self {
            preloaded: cached_kind(choice),
            sets: Vec::new(),
        }
    }
}

const fn cached_kind(choice: CachedChoice) -> Option<ClientKind> {
    match choice {
        CachedChoice::None => None,
        CachedChoice::AzureOpenAi => Some(ClientKind::AzureOpenAi),
        CachedChoice::AsyncAzureOpenAi => Some(ClientKind::AsyncAzureOpenAi),
        CachedChoice::OpenAi => Some(ClientKind::OpenAi),
        CachedChoice::AsyncOpenAi => Some(ClientKind::AsyncOpenAi),
        CachedChoice::Other => Some(ClientKind::Other),
    }
}

impl ClientCache for Cache {
    fn get(&self, _identity: &ClientIdentity) -> Option<ClientKind> {
        self.preloaded
    }

    fn set(&mut self, identity: ClientIdentity, kind: ClientKind, owned: bool) {
        self.sets.push((identity, kind, owned));
    }
}
