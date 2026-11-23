use super::answer::{AdapterError, AdapterUnavailable, answered};
use super::client::NativeClients;
use super::convert::{GenaiCall, NativeErrorPayload, SessionWire, parse};
use super::error::{GenaiConfigError, SessionError, payload_error};
use super::logging::{SilentSink, logging_env, logging_state};
use super::provider::NativeContentConfig;
use super::stream::StreamKind;
use betterlite_config::env_integer::timeout::RequestTimeout;
use betterlite_config::loader::startup::process_environment;
use betterlite_config::sections::Section as _;
use betterlite_config::sections::studio_identity::Raw as StudioRaw;
use betterlite_config::sections::studio_identity::Settings as StudioSettings;
use betterlite_config::sections::vertex_credential_sources::Raw as VertexRaw;
use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_schema_shaping::Raw as SchemaRaw;
use betterlite_config::sections::vertex_schema_shaping::Settings as SchemaSettings;
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
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::openai_base_config::CatalogCapabilities;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_vertex::credential_sources::sdk::{
    CredentialClock, GoogleAuth, HttpPolicy, SystemClock,
};
use betterlite_vertex::google_native_content::capabilities::NativeProvider;
use betterlite_vertex::google_native_content::response::GenerateContentResponse;
use betterlite_vertex::model_policy::partners::is_vertex_partner_model;
use pyo3::PyErr;
use secrecy::SecretString;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;
mod tests;
// The marker each entry point carries: litellm.google_genai.main's four functions, and the stream flag each one sets.
#[derive(Clone, Copy)]
pub(crate) enum Entry {
    Blocking,
    Await,
    BlockingStream,
    AwaitStream,
}
impl Entry {
    pub(crate) const fn call_type(self) -> &'static str {
        match self {
            Self::Blocking => "generate_content",
            Self::Await => "agenerate_content",
            Self::BlockingStream => "generate_content_stream",
            Self::AwaitStream => "agenerate_content_stream",
        }
    }
}
// What a finished call hands back: the response text Python loads, or the stream it iterates.
pub(crate) enum Outcome {
    Text(String),
    Stream(StreamKind),
}
// Everything one Python session owns: the environment snapshot, the settings read from it and the clients every call goes through.
pub(crate) struct GenaiCore {
    environment: BTreeMap<String, String>,
    studio: StudioSettings,
    vertex: VertexSettings,
    schema: SchemaSettings,
    request_timeout: RequestTimeout,
    catalog: Arc<ModelCatalog>,
    registry: JSONProviderRegistry,
    known_model_names: BTreeSet<String>,
    custom_providers: BTreeSet<String>,
    capabilities: CatalogCapabilities,
    clients: NativeClients,
    clock: Arc<dyn CredentialClock>,
    redactor: Redactor,
}
impl GenaiCore {
    pub(crate) fn build(text: &str) -> Result<Self, SessionError> {
        let wire: SessionWire = parse(text)?;
        let environment = wire.environment.unwrap_or_else(process_environment);
        let mut studio = StudioRaw::default()
            .convert(&environment)
            .map_err(|source| SessionError::Settings { source })?;
        // litellm.api_key: the last studio key LiteLLM reads, after the GOOGLE_API_KEY and GEMINI_API_KEY environment keys
        studio.generic_api_key = wire
            .api_key
            .map(SecretString::from)
            .or(studio.generic_api_key);
        let vertex = VertexRaw::default()
            .convert(&environment)
            .map_err(|source| SessionError::Settings { source })?;
        let schema = SchemaRaw::default()
            .convert(&environment)
            .map_err(|source| SessionError::Settings { source })?;
        let request_timeout = RequestTimeout::resolve(&environment)
            .map_err(|source| SessionError::Timeout { source })?;
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
            .map_err(|source| SessionError::Redactor { source })?;
        let clients = NativeClients::build(&environment)?;
        Ok(Self {
            environment,
            studio,
            vertex,
            schema,
            request_timeout,
            catalog: BundledCatalog::default().get(),
            registry: JSONProviderRegistry::load(),
            known_model_names: BTreeSet::new(),
            custom_providers: BTreeSet::new(),
            capabilities: CatalogCapabilities {
                chat_models: BTreeSet::new(),
                text_models: BTreeSet::new(),
            },
            clients,
            clock: Arc::new(SystemClock),
            redactor,
        })
    }
    // one call: the request B3489 reads, the provider slots it dispatches between, and the logging state it records into
    pub(crate) async fn call(&self, call: GenaiCall, entry: Entry) -> Result<Outcome, PyErr> {
        let timeout = call.timeout()?;
        let request = request_of(&call, timeout);
        let gemini = self.config(NativeProvider::Studio);
        let vertex = self.config(NativeProvider::Vertex);
        let configs = GenerateContentProviderConfigs {
            gemini: &gemini,
            vertex: &vertex,
            is_vertex_partner_model,
        };
        let resolution = self.resolution();
        let executor = AdapterUnavailable;
        let deps = GenerateContentDeps {
            configs: &configs,
            resolution: &resolution,
            capabilities: &self.capabilities,
            limits: SchemaLimits::default(),
            source: &self.clients,
            supplied_client: None,
            executor: &executor,
            stream: StreamSettings::default(),
            request_timeout: self.request_timeout,
            redactor: &self.redactor,
        };
        let mut state = logging_state(&call, entry, &self.redactor)?;
        let sink = SilentSink;
        let env = logging_env(&self.redactor, &sink);
        let mut log = LogContext {
            state: &mut state,
            env: &env,
        };
        let answer = dispatch(&deps, &request, &mut log, entry)
            .await
            .map_err(|failure| payload_error(&NativeErrorPayload::from_class(&failure.class)))?;
        answered(answer)
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
// the request the four entry points read, field for field
fn request_of(call: &GenaiCall, timeout: Option<Duration>) -> GenerateContentRequest<'_> {
    GenerateContentRequest {
        model: &call.model,
        contents: &call.contents,
        config: call.config.as_ref(),
        tools: call.tools.as_ref(),
        extra_headers: call.extra_headers.as_ref(),
        extra_query: call.extra_query.as_ref(),
        extra_body: call.extra_body.as_ref(),
        timeout,
        custom_llm_provider: call.custom_llm_provider.as_deref(),
        kwargs: &call.kwargs,
    }
}
// one of the four entry points, all of which are async and end with the same setup and dispatch
async fn dispatch(
    deps: &GenerateContentDeps<'_, NativeContentConfig, NativeClients, AdapterUnavailable>,
    request: &GenerateContentRequest<'_>,
    log: &mut LogContext<'_, '_>,
    entry: Entry,
) -> Result<
    GenerateContentAnswer<GenerateContentResponse>,
    GenerateContentFailure<GenaiConfigError, AdapterError>,
> {
    match entry {
        Entry::Blocking => generate_content(deps, request, log).await,
        Entry::Await => agenerate_content(deps, request, log).await,
        Entry::BlockingStream => generate_content_stream(deps, request, log).await,
        Entry::AwaitStream => agenerate_content_stream(deps, request, log).await,
    }
}
