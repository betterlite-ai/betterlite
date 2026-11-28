pub mod call;
pub mod clients;
pub mod error;
pub mod provider;
mod tests;

use self::clients::NativeClients;
use self::error::GenaiConfigError;
use self::provider::NativeContentConfig;
use crate::credential_sources::aws::AwsCredentialsSource;
use crate::credential_sources::sdk::{CredentialClock, GoogleAuth, HttpPolicy};
use crate::google_native_content::capabilities::NativeProvider;
use crate::google_native_content::response::GenerateContentResponse;
use crate::model_policy::partners::is_vertex_partner_model;
use betterlite_config::env_integer::timeout::RequestTimeout;
use betterlite_config::sections::studio_identity::Settings as StudioSettings;
use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_schema_shaping::Settings as SchemaSettings;
use betterlite_core::api::genai::chat_call::CompletionExecutor;
use betterlite_core::api::genai::generate_content::async_stream::agenerate_content_stream;
use betterlite_core::api::genai::generate_content::async_unary::agenerate_content;
use betterlite_core::api::genai::generate_content::error::GenerateContentFailure;
use betterlite_core::api::genai::generate_content::stream::generate_content_stream;
use betterlite_core::api::genai::generate_content::unary::generate_content;
use betterlite_core::api::genai::generate_content::{
    GenerateContentAnswer, GenerateContentDeps, GenerateContentRequest,
};
use betterlite_core::api::genai_http::stream::StreamSettings;
use betterlite_core::api::vector_store_http_write::service::LogContext;
use betterlite_core::genai_setup::config::GenerateContentProviderConfigs;
use betterlite_core::schema::SchemaLimits;
use betterlite_logging::logging_pre_post::env::{DebugSettings, GlobalFlags, LogSink, PrePostEnv};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::openai_base_config::CatalogCapabilities;
use betterlite_redact::secret::Redactor;
use chrono::Utc;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

// The marker each entry point carries: litellm.google_genai.main's four functions, and the stream flag each one sets
#[derive(Clone, Copy)]
pub enum Entry {
    Blocking,
    Await,
    BlockingStream,
    AwaitStream,
}

impl Entry {
    #[must_use]
    pub const fn call_type(self) -> &'static str {
        match self {
            Self::Blocking => "generate_content",
            Self::Await => "agenerate_content",
            Self::BlockingStream => "generate_content_stream",
            Self::AwaitStream => "agenerate_content_stream",
        }
    }
}

// What a caller resolves before the session exists; the registry, the empty name sets and the empty capabilities the session builds itself
pub struct SessionParts {
    pub environment: BTreeMap<String, String>,
    pub studio: StudioSettings,
    pub vertex: VertexSettings,
    pub schema: SchemaSettings,
    pub request_timeout: RequestTimeout,
    pub catalog: Arc<ModelCatalog>,
    pub clients: NativeClients,
    pub clock: Arc<dyn CredentialClock>,
    pub redactor: Redactor,
    pub aws: Arc<dyn AwsCredentialsSource>,
}

// Everything one generate content call owns: the environment snapshot, the settings read from it and the clients every call goes through
pub struct GenaiSession {
    pub environment: BTreeMap<String, String>,
    pub studio: StudioSettings,
    pub vertex: VertexSettings,
    pub schema: SchemaSettings,
    pub request_timeout: RequestTimeout,
    pub catalog: Arc<ModelCatalog>,
    pub registry: JSONProviderRegistry,
    pub known_model_names: BTreeSet<String>,
    pub custom_providers: BTreeSet<String>,
    pub capabilities: CatalogCapabilities,
    pub clients: NativeClients,
    pub clock: Arc<dyn CredentialClock>,
    // a caller that logs the call needs the same scrubber the handler redacts with
    pub redactor: Redactor,
    pub aws: Arc<dyn AwsCredentialsSource>,
}

impl GenaiSession {
    #[must_use]
    pub fn new(parts: SessionParts) -> Self {
        Self {
            environment: parts.environment,
            studio: parts.studio,
            vertex: parts.vertex,
            schema: parts.schema,
            request_timeout: parts.request_timeout,
            catalog: parts.catalog,
            registry: JSONProviderRegistry::load(),
            known_model_names: BTreeSet::new(),
            custom_providers: BTreeSet::new(),
            capabilities: CatalogCapabilities {
                chat_models: BTreeSet::new(),
                text_models: BTreeSet::new(),
            },
            clients: parts.clients,
            clock: parts.clock,
            redactor: parts.redactor,
            aws: parts.aws,
        }
    }

    // one call: the request B3489 reads, the provider slots it dispatches between, and the logging state it records into
    pub async fn run<Executor>(
        &self,
        request: &GenerateContentRequest<'_>,
        executor: &Executor,
        log: &mut LogContext<'_, '_>,
        entry: Entry,
    ) -> Result<
        GenerateContentAnswer<GenerateContentResponse>,
        GenerateContentFailure<GenaiConfigError, Executor::Error>,
    >
    where
        Executor: CompletionExecutor + Sync,
    {
        let gemini = self.config(NativeProvider::Studio);
        let vertex = self.config(NativeProvider::Vertex);
        let configs = GenerateContentProviderConfigs {
            gemini: &gemini,
            vertex: &vertex,
            is_vertex_partner_model,
        };
        let resolution = self.resolution();
        let deps = GenerateContentDeps {
            configs: &configs,
            resolution: &resolution,
            capabilities: &self.capabilities,
            limits: SchemaLimits::default(),
            source: &self.clients,
            supplied_client: None,
            executor,
            stream: StreamSettings::default(),
            request_timeout: self.request_timeout,
            redactor: &self.redactor,
        };
        dispatch(&deps, request, log, entry).await
    }

    // the collaborating provider config one resolved provider runs against
    fn config(&self, provider: NativeProvider) -> NativeContentConfig {
        NativeContentConfig {
            provider,
            catalog: Arc::clone(&self.catalog),
            studio: self.studio.clone(),
            vertex: self.vertex.clone(),
            schema: self.schema.clone(),
            google: GoogleAuth {
                client: self.clients.default.clone(),
                policy: HttpPolicy::default(),
            },
            clock: Arc::clone(&self.clock),
            aws: Arc::clone(&self.aws),
        }
    }

    // the context get_llm_provider reads: this session's catalog and environment, and no global credentials of its own
    fn resolution(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.catalog,
            known_model_names: &self.known_model_names,
            custom_providers: &self.custom_providers,
            environment: &self.environment,
            registry: self.registry.clone(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }
}

// the pre/post env the handler logs through; a session has no callbacks, sentry or logger function of its own
#[must_use]
pub fn logging_env<'env>(redactor: &'env Redactor, sink: &'env dyn LogSink) -> PrePostEnv<'env> {
    PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: sink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: Utc::now().fixed_offset(),
    }
}

// one of the four entry points, all of which are async and end with the same setup and dispatch
async fn dispatch<Executor>(
    deps: &GenerateContentDeps<'_, NativeContentConfig, NativeClients, Executor>,
    request: &GenerateContentRequest<'_>,
    log: &mut LogContext<'_, '_>,
    entry: Entry,
) -> Result<
    GenerateContentAnswer<GenerateContentResponse>,
    GenerateContentFailure<GenaiConfigError, Executor::Error>,
>
where
    Executor: CompletionExecutor + Sync,
{
    match entry {
        Entry::Blocking => generate_content(deps, request, log).await,
        Entry::Await => agenerate_content(deps, request, log).await,
        Entry::BlockingStream => generate_content_stream(deps, request, log).await,
        Entry::AwaitStream => agenerate_content_stream(deps, request, log).await,
    }
}
