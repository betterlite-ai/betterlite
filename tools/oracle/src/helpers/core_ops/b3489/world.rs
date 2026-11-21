use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};
use std::time::Duration;

use betterlite_core::api::genai::chat_call::{
    CompletionCall, CompletionExecutor, CompletionOutput,
};
use betterlite_core::api::genai_http::config::{AuthCall, AuthTarget, GenerateContentHttpConfig};
use betterlite_core::api::genai_http::service::{
    ClientSource, GenAiRequest, GenAiTransport, WireReply,
};
use betterlite_core::api::vector_store_http_write::error::{
    BaseLlmException, ClientError, HeaderMap, header_map,
};
use betterlite_core::genai_setup::config::{
    GenerateContentProviderConfig, GenerateContentProviderConfigs,
    GenerateContentRequest as SetupRequest,
};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_prompts::arguments::json::{Json, Object};

#[derive(Debug, thiserror::Error)]
#[error("the oracle does not build provider requests")]
pub(super) struct ConfigError;

#[derive(Debug, thiserror::Error)]
#[error("the oracle does not run completions")]
pub(super) struct ExecutorError;

#[derive(Default)]
pub(super) struct Config;

impl GenerateContentProviderConfig for Config {
    type Error = ConfigError;

    fn map_optional_params(&self, config: &Object, _model: &str) -> Object {
        config.clone()
    }

    fn transform_request(&self, _request: &SetupRequest<'_>) -> Result<Json, ConfigError> {
        Err(ConfigError)
    }

    fn request_top_level_fields(&self) -> &'static [&'static str] {
        &[]
    }
}

impl GenerateContentHttpConfig for Config {
    type Response = Json;

    fn auth_token_and_url(
        &self,
        _call: &AuthCall<'_>,
    ) -> impl Future<Output = Result<AuthTarget, ConfigError>> + Send {
        ready(Err(ConfigError))
    }

    fn decode_response(&self, response: &UpstreamResponse) -> Result<Json, BaseLlmException> {
        Err(self.error_class(
            "the oracle does not decode provider responses",
            response.status,
            header_map(&response.headers),
        ))
    }

    fn error_class(&self, message: &str, status_code: u16, headers: HeaderMap) -> BaseLlmException {
        BaseLlmException {
            status_code,
            message: message.to_owned(),
            headers,
            status_code_is_synthesized: false,
        }
    }
}

#[derive(Default)]
pub(super) struct Source;

pub(super) struct StubClient;

impl ClientSource for Source {
    type Client = StubClient;

    fn client(&self, _ssl_verify: Option<&Json>) -> Result<StubClient, ClientError> {
        Ok(StubClient)
    }
}

impl GenAiTransport for StubClient {
    fn default_timeout(&self) -> Duration {
        Duration::from_secs(60)
    }

    fn send(
        &self,
        _request: GenAiRequest,
        _streaming: bool,
    ) -> impl Future<Output = Result<WireReply, TransportError>> + Send {
        ready(Err(TransportError::HeadersTooLarge { limit: 0 }))
    }
}

#[derive(Default)]
pub(super) struct Executor;

impl CompletionExecutor for Executor {
    type Error = ExecutorError;

    fn completion(&self, _call: CompletionCall) -> Result<CompletionOutput, ExecutorError> {
        Err(ExecutorError)
    }

    fn acompletion(
        &self,
        _call: CompletionCall,
    ) -> impl Future<Output = Result<CompletionOutput, ExecutorError>> + Send {
        ready(Err(ExecutorError))
    }
}

// Every collaborator the entry points take, over stubs that fail when a case reaches a real provider
#[derive(Default)]
pub(super) struct World {
    pub(super) catalog: ModelCatalog,
    pub(super) names: BTreeSet<String>,
    pub(super) environment: BTreeMap<String, String>,
    pub(super) config: Config,
    pub(super) source: Source,
    pub(super) executor: Executor,
}

impl World {
    pub(super) fn resolution(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.catalog,
            known_model_names: &self.names,
            custom_providers: &self.names,
            environment: &self.environment,
            registry: JSONProviderRegistry::default(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }

    pub(super) fn configs(&self) -> GenerateContentProviderConfigs<'_, Config> {
        GenerateContentProviderConfigs {
            gemini: &self.config,
            vertex: &self.config,
            is_vertex_partner_model: |_model| false,
        }
    }
}
