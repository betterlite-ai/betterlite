use std::collections::BTreeMap;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;

use secrecy::{ExposeSecret as _, SecretString};

use crate::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use crate::credentials::cache::RefreshProviderCache;
use crate::credentials::source::{
    CallableTokenProvider, CredentialSources, ProviderFailure, SourceFailure, TokenReply,
};
use crate::endpoints::get_client::{
    ClientCache, GetClientRequest, get_azure_openai_client, initialize_request,
};
use crate::endpoints::initialize::{EndpointError, initialize_azure_sdk_client};
use crate::endpoints::projection::{
    AzureClientInputs, CallableProvider, ClientIdentity, ClientKind, ClientOutcome, LiteLlmParams,
    SessionKind, SuppliedClient,
};
use crate::oidc_exchange::OidcExchangeInput;
use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::azure_endpoints::Settings;

pub(crate) const DEFAULT_VERSION: &str = "2025-02-01-preview";
pub(crate) const DEFAULT_API_BASE: &str = "https://example.openai.azure.com";

pub(crate) fn revealed(secret: &SecretString) -> &str {
    secret.expose_secret()
}

#[derive(Debug, Default, Clone)]
pub(crate) struct Env(pub(crate) BTreeMap<String, String>);

impl EnvironmentLookup for Env {
    fn lookup(&self, name: &str) -> Option<String> {
        self.0.get(name).cloned()
    }
}

#[derive(Debug, Clone)]
pub(crate) enum Script {
    Token(String),
    Raise { class: String, message: String },
}

pub(crate) struct Provider {
    script: Script,
}

impl CallableTokenProvider for Provider {
    fn token(&self) -> Result<TokenReply, ProviderFailure> {
        match &self.script {
            Script::Token(token) => Ok(TokenReply::Text(SecretString::from(token.clone()))),
            Script::Raise { class, message } => Err(ProviderFailure {
                class: class.clone(),
                message: message.clone(),
            }),
        }
    }
}

pub(crate) fn provider(script: Script) -> Arc<dyn CallableTokenProvider> {
    let provider = Provider { script };
    Arc::new(provider)
}

pub(crate) fn callable(script: Script, identity: u64) -> CallableProvider {
    CallableProvider {
        provider: provider(script),
        identity,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SourceCall {
    ClientSecret {
        tenant_id: String,
        client_id: String,
        scope: String,
    },
    UsernamePassword {
        client_id: String,
        username: String,
        scope: String,
    },
    Refresh {
        scope: String,
    },
    Oidc {
        azure_ad_token: String,
        client_id: Option<String>,
        tenant_id: Option<String>,
        scope: Option<String>,
    },
}

type Reply<'data> = Result<Arc<dyn CallableTokenProvider + 'data>, SourceFailure>;

#[derive(Default)]
pub(crate) struct Sources {
    scripts: BTreeMap<&'static str, Script>,
    calls: Mutex<Vec<SourceCall>>,
}

impl Sources {
    pub(crate) fn set_script(&mut self, name: &'static str, script: Script) {
        self.scripts.insert(name, script);
    }

    pub(crate) fn calls(&self) -> Vec<SourceCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn scripted(&self, call: SourceCall) -> Result<String, SourceFailure> {
        let (name, default_token) = match &call {
            SourceCall::ClientSecret { .. } => ("client_secret", "client-secret-token"),
            SourceCall::UsernamePassword { .. } => ("username_password", "username-token"),
            SourceCall::Refresh { .. } => ("refresh", "refresh-token"),
            SourceCall::Oidc { .. } => ("oidc", "oidc-token"),
        };
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
        match self.scripts.get(name) {
            None => Ok(default_token.to_owned()),
            Some(Script::Token(token)) => Ok(token.clone()),
            Some(Script::Raise { class, message }) => Err(SourceFailure {
                class: class.clone(),
                status: None,
                message: message.clone(),
            }),
        }
    }

    fn token_provider(&self, call: SourceCall) -> Reply<'_> {
        self.scripted(call)
            .map(|token| provider(Script::Token(token)))
    }
}

impl CredentialSources for Sources {
    fn client_secret(&self, record: &ClientSecretRecord) -> Reply<'_> {
        self.token_provider(SourceCall::ClientSecret {
            tenant_id: record.tenant_id.clone(),
            client_id: record.client_id.clone(),
            scope: record.scope.clone(),
        })
    }

    fn username_password(&self, record: &UsernamePasswordRecord) -> Reply<'_> {
        self.token_provider(SourceCall::UsernamePassword {
            client_id: record.client_id.clone(),
            username: record.username.clone(),
            scope: record.scope.clone(),
        })
    }

    fn sdk_refresh(&self, scope: &str) -> Reply<'_> {
        self.token_provider(SourceCall::Refresh {
            scope: scope.to_owned(),
        })
    }

    fn default_credential(&self, _scope: &str) -> Option<Arc<dyn CallableTokenProvider + '_>> {
        None
    }

    fn oidc(&self, input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        self.scripted(SourceCall::Oidc {
            azure_ad_token: input.azure_ad_token.expose_secret().to_owned(),
            client_id: input.azure_client_id.clone(),
            tenant_id: input.azure_tenant_id.clone(),
            scope: input.scope.clone(),
        })
        .map(SecretString::from)
    }
}

#[derive(Default)]
pub(crate) struct Inputs {
    pub(crate) params: LiteLlmParams,
    pub(crate) settings: Settings,
    pub(crate) environment: Env,
    pub(crate) sources: Sources,
}

impl Inputs {
    pub(crate) fn set_script(&mut self, name: &'static str, script: Script) {
        self.sources.set_script(name, script);
    }

    #[must_use]
    pub(crate) fn with_tenant(mut self, value: &str) -> Self {
        self.params.auth.tenant_id = Some(value.to_owned());
        self
    }

    #[must_use]
    pub(crate) fn with_client_id(mut self, value: &str) -> Self {
        self.params.auth.client_id = Some(value.to_owned());
        self
    }

    #[must_use]
    pub(crate) fn with_client_secret(mut self, value: &SecretString) -> Self {
        self.params.auth.client_secret = Some(value.clone());
        self
    }

    #[must_use]
    pub(crate) fn with_username(mut self, value: &str) -> Self {
        self.params.auth.azure_username = Some(value.to_owned());
        self
    }

    #[must_use]
    pub(crate) fn with_password(mut self, value: &SecretString) -> Self {
        self.params.auth.azure_password = Some(value.clone());
        self
    }

    #[must_use]
    pub(crate) fn with_scope(mut self, value: &str) -> Self {
        self.params.auth.azure_scope = Some(value.to_owned());
        self
    }

    #[must_use]
    pub(crate) fn with_ad_token(mut self, value: &SecretString) -> Self {
        self.params.azure_ad_token = Some(value.clone());
        self
    }

    #[must_use]
    pub(crate) fn with_max_retries(mut self, value: Option<i64>) -> Self {
        self.params.max_retries = value;
        self
    }

    #[must_use]
    pub(crate) fn with_timeout(mut self, value: Option<f64>) -> Self {
        self.params.timeout = value;
        self
    }

    #[must_use]
    pub(crate) fn with_default_retries(mut self, value: i64) -> Self {
        self.settings.default_max_retries = value;
        self
    }
}

pub(crate) struct Run {
    pub(crate) api_version: Option<String>,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) api_base: Option<String>,
    pub(crate) provider: Option<CallableProvider>,
    pub(crate) supplied: Option<SuppliedClient>,
    pub(crate) is_async: bool,
    pub(crate) session: SessionKind,
    pub(crate) refresh_enabled: bool,
}

impl Default for Run {
    fn default() -> Self {
        Self {
            api_version: None,
            api_key: None,
            api_base: Some(DEFAULT_API_BASE.to_owned()),
            provider: None,
            supplied: None,
            is_async: false,
            session: SessionKind::Dedicated,
            refresh_enabled: false,
        }
    }
}

impl Run {
    pub(crate) fn for_client() -> Self {
        Self::default().with_version(Some("2024-02-01"))
    }

    #[must_use]
    pub(crate) fn with_version(mut self, version: Option<&str>) -> Self {
        self.api_version = version.map(str::to_owned);
        self
    }

    #[must_use]
    pub(crate) fn with_key(mut self, key: &SecretString) -> Self {
        self.api_key = Some(key.clone());
        self
    }

    #[must_use]
    pub(crate) fn with_provider(mut self, provider: &CallableProvider) -> Self {
        self.provider = Some(provider.clone());
        self
    }

    #[must_use]
    pub(crate) fn with_supplied(mut self, supplied: SuppliedClient) -> Self {
        self.supplied = Some(supplied);
        self
    }

    #[must_use]
    pub(crate) fn with_api_base(mut self, base: &str) -> Self {
        self.api_base = Some(base.to_owned());
        self
    }

    #[must_use]
    pub(crate) fn async_shared(mut self) -> Self {
        self.is_async = true;
        self.session = SessionKind::Shared;
        self
    }

    #[must_use]
    pub(crate) fn refresh(mut self) -> Self {
        self.refresh_enabled = true;
        self
    }
}

pub(crate) fn initialize<'data>(inputs: &'data Inputs, run: &Run) -> AzureClientInputs<'data> {
    initialize_result(inputs, run).unwrap()
}

pub(crate) fn initialize_result<'data>(
    inputs: &'data Inputs,
    run: &Run,
) -> Result<AzureClientInputs<'data>, EndpointError> {
    let mut refresh = RefreshProviderCache::new(&inputs.sources);
    initialize_azure_sdk_client(
        &initialize_request(&client_request(inputs, run)),
        &mut refresh,
    )
}

pub(crate) fn get_client<'data>(
    inputs: &'data Inputs,
    run: &Run,
    cache: &mut dyn ClientCache,
) -> Result<ClientOutcome<'data>, EndpointError> {
    let mut refresh = RefreshProviderCache::new(&inputs.sources);
    get_azure_openai_client(&client_request(inputs, run), cache, &mut refresh)
}

fn client_request<'data>(inputs: &'data Inputs, run: &Run) -> GetClientRequest<'data> {
    GetClientRequest {
        api_key: run.api_key.clone(),
        api_base: run.api_base.clone(),
        api_version: run.api_version.clone(),
        model: Some("gpt-4o".to_owned()),
        is_async: run.is_async,
        params: inputs.params.clone(),
        provider: run.provider.clone(),
        supplied: run.supplied.clone(),
        settings: &inputs.settings,
        environment: &inputs.environment,
        refresh_enabled: run.refresh_enabled,
        session: run.session,
        sources: &inputs.sources,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CacheSet {
    pub(crate) identity: ClientIdentity,
    pub(crate) kind: ClientKind,
    pub(crate) owned: bool,
}

pub(crate) struct Cache {
    pub(crate) preloaded: Option<ClientKind>,
    pub(crate) sets: Vec<CacheSet>,
}

impl Cache {
    pub(crate) fn empty() -> Self {
        Self {
            preloaded: None,
            sets: Vec::new(),
        }
    }
}

impl ClientCache for Cache {
    fn get(&self, _identity: &ClientIdentity) -> Option<ClientKind> {
        self.preloaded
    }

    fn set(&mut self, identity: ClientIdentity, kind: ClientKind, owned: bool) {
        self.sets.push(CacheSet {
            identity,
            kind,
            owned,
        });
    }
}

pub(crate) struct KeyedCache {
    entries: HashMap<ClientIdentity, ClientKind>,
}

impl KeyedCache {
    pub(crate) fn empty() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }
}

impl ClientCache for KeyedCache {
    fn get(&self, identity: &ClientIdentity) -> Option<ClientKind> {
        self.entries.get(identity).copied()
    }

    fn set(&mut self, identity: ClientIdentity, kind: ClientKind, _owned: bool) {
        self.entries.insert(identity, kind);
    }
}
