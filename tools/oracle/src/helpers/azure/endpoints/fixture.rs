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
    CallableProvider {
        provider: Arc::new(Provider { script }),
        identity,
    }
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

    fn scripted(&self, request: &Request<'_>) -> Result<String, SourceFailure> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.call());
        let (script, default_token) = match request {
            Request::ClientSecret(_) => (&self.scripts.client_secret, "client-secret-token"),
            Request::UsernamePassword(_) => (&self.scripts.username_password, "username-token"),
            Request::Refresh(_) => (&self.scripts.refresh, "refresh-token"),
            Request::Oidc(_) => (&self.scripts.oidc, "oidc-token"),
        };
        match script {
            None => Ok(default_token.to_owned()),
            Some(Script::Token { token }) => Ok(token.clone()),
            Some(Script::Raise {
                class_name,
                message,
            }) => Err(SourceFailure {
                class: class_name.clone(),
                status: None,
                message: message.clone(),
            }),
        }
    }

    fn token_provider(&self, request: &Request<'_>) -> Reply<'_> {
        let token = self.scripted(request)?;
        let provider: Arc<dyn CallableTokenProvider> = Arc::new(Provider {
            script: Script::Token { token },
        });
        Ok(provider)
    }
}

type Reply<'data> = Result<Arc<dyn CallableTokenProvider + 'data>, SourceFailure>;

enum Request<'data> {
    ClientSecret(&'data ClientSecretRecord),
    UsernamePassword(&'data UsernamePasswordRecord),
    Refresh(&'data str),
    Oidc(&'data OidcExchangeInput),
}

impl Request<'_> {
    fn call(&self) -> Value {
        match self {
            Self::ClientSecret(record) => json!({
                "source": "client_secret",
                "tenant_id": record.tenant_id,
                "client_id": record.client_id,
                "client_secret": record.client_secret.expose_secret(),
                "scope": record.scope,
            }),
            Self::UsernamePassword(record) => json!({
                "source": "username_password",
                "client_id": record.client_id,
                "azure_username": record.username,
                "azure_password": record.password.expose_secret(),
                "scope": record.scope,
            }),
            Self::Refresh(scope) => json!({"source": "refresh", "scope": scope}),
            Self::Oidc(input) => json!({
                "source": "oidc",
                "azure_ad_token": input.azure_ad_token.expose_secret(),
                "azure_client_id": input.azure_client_id,
                "azure_tenant_id": input.azure_tenant_id,
                "scope": input.scope,
            }),
        }
    }
}

impl CredentialSources for Sources {
    fn client_secret(&self, record: &ClientSecretRecord) -> Reply<'_> {
        self.token_provider(&Request::ClientSecret(record))
    }

    fn username_password(&self, record: &UsernamePasswordRecord) -> Reply<'_> {
        self.token_provider(&Request::UsernamePassword(record))
    }

    fn sdk_refresh(&self, scope: &str) -> Reply<'_> {
        self.token_provider(&Request::Refresh(scope))
    }

    fn default_credential(&self, _scope: &str) -> Option<Arc<dyn CallableTokenProvider + '_>> {
        None
    }

    fn oidc(&self, input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        self.scripted(&Request::Oidc(input)).map(SecretString::from)
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
