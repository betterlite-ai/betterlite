pub mod budget;
pub mod langfuse;

pub(crate) mod tests;

use std::sync::Arc;
use std::time::{Duration, TryFromFloatSecsError};

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::langfuse_settings::Settings;
use betterlite_http::proxy::ProxiedClient;
use tokio::runtime::Handle;
use tokio_util::task::TaskTracker;

use crate::langfuse_channels::{LangfuseChannelError, LangfuseChannelRequest};
use crate::langfuse_rest::cache::Clock;
use crate::langfuse_rest::client::{
    LangfuseApiClient, LangfuseClientConfig, LangfuseClientError, build_langfuse_client,
};
use crate::langfuse_settings::{
    CredentialRequest, Credentials, LangfuseSettingsError, export, release,
};

use self::budget::{LangfuseBudgetError, LangfuseClientBudget};

// the litellm_settings values the generation assembly reads
#[derive(Clone, Debug, Default)]
pub struct AssemblyDefaults {
    pub default_tags: Vec<String>,
    pub update_trace_keys: bool,
    pub redact_user_api_key_info: bool,
}

// everything a LangFuseLogger is built from, injected so no logger reaches for process-wide state
pub struct LangfuseServices {
    pub settings: Settings,
    pub environment: Arc<dyn EnvironmentLookup + Send + Sync>,
    pub http: ProxiedClient,
    pub budget: LangfuseClientBudget,
    pub tasks: TaskTracker,
    pub runtime: Handle,
    pub assembly: AssemblyDefaults,
    pub clients: Arc<dyn LangfuseClientBuilder>,
}

// litellm.integrations.langfuse.langfuse_sdk.build_langfuse_client, behind a seam a test can fail
pub trait LangfuseClientBuilder: Send + Sync {
    fn build(&self, config: LangfuseClientConfig)
    -> Result<LangfuseApiClient, LangfuseClientError>;
}

pub struct HttpClientBuilder {
    pub http: ProxiedClient,
    pub clock: Arc<dyn Clock>,
    pub tasks: TaskTracker,
}

impl LangfuseClientBuilder for HttpClientBuilder {
    fn build(
        &self,
        config: LangfuseClientConfig,
    ) -> Result<LangfuseApiClient, LangfuseClientError> {
        build_langfuse_client(
            config,
            Some(self.http.clone()),
            Arc::clone(&self.clock),
            self.tasks.clone(),
        )
    }
}

// the LangFuseLogger.__init__ arguments once the handler has resolved the request's credentials
#[derive(Clone, Copy)]
pub struct LangfuseLoggerRequest<'request> {
    pub credentials: CredentialRequest<'request>,
    pub environment: Option<&'request str>,
    pub flush_interval: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum LangfuseLoggerError {
    #[error(transparent)]
    Settings(#[from] LangfuseSettingsError),
    #[error(transparent)]
    Budget(#[from] LangfuseBudgetError),
    #[error(transparent)]
    Channel(#[from] LangfuseChannelError),
    #[error(transparent)]
    Client(#[from] LangfuseClientError),
    #[error("LANGFUSE_TIMEOUT='{raw}' is not a number of seconds above 0")]
    Timeout {
        raw: String,
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error("LANGFUSE_PROMPT_CACHE_DEFAULT_TTL_SECONDS is beyond the range of a duration")]
    PromptCacheTtl {
        #[source]
        source: TryFromFloatSecsError,
    },
}

// safe_init_langfuse_client's channel request, built the same way by a test that seeds the channel
fn channel_request(
    settings: &Settings,
    credentials: &Credentials,
    environment: Option<String>,
    flush_interval: u64,
    mock: bool,
) -> LangfuseChannelRequest {
    LangfuseChannelRequest {
        public_key: credentials.public_key.clone().unwrap_or_default(),
        secret_key: credentials.secret_key.clone().unwrap_or_default(),
        base_url: credentials.host.clone(),
        environment,
        release: release(settings),
        flush_interval,
        mock_mode: mock,
    }
}

// build_langfuse_client's arguments; LiteLLM raises out of the logger when a value cannot be a duration
fn client_config(
    settings: &Settings,
    credentials: &Credentials,
) -> Result<LangfuseClientConfig, LangfuseLoggerError> {
    let timeout =
        Duration::try_from_secs_f64(export::timeout_seconds(settings)?).map_err(|source| {
            LangfuseLoggerError::Timeout {
                raw: settings.timeout.clone().unwrap_or_default(),
                source,
            }
        })?;
    let prompt_cache_ttl = Duration::try_from_secs_f64(export::prompt_cache_ttl_seconds(settings))
        .map_err(|source| LangfuseLoggerError::PromptCacheTtl { source })?;
    Ok(LangfuseClientConfig {
        public_key: credentials.public_key.clone(),
        secret_key: credentials.secret_key.clone(),
        base_url: credentials.host.clone(),
        timeout,
        prompt_cache_ttl,
    })
}
