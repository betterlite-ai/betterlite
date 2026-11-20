use std::future::{Future, ready};
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_anthropic::count_tokens::handler::CountTokensTransport;
use betterlite_azure::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use betterlite_azure::credentials::source::{
    CallableTokenProvider, CredentialSources, SourceFailure,
};
use betterlite_azure::oidc_exchange::OidcExchangeInput;
use betterlite_azure::passthrough::config::AzurePassthroughConfig;
use betterlite_azure::passthrough::logging::{NoChatReplyProjection, NoChatStreamAssembly};
use betterlite_config::sections::azure_credentials::DeploymentSettings;
use betterlite_foundry::mai_images::reply::{ImageLogging, PostCall};
use betterlite_foundry::passthrough::logging::FoundryPassthroughLogging;
use betterlite_foundry::passthrough::plan::FoundryPassthroughPlan;
use betterlite_foundry::vector_stores::embedding::{EmbeddingCallError, QueryEmbeddingExecutor};
use betterlite_foundry::vector_stores::environment::Params as VectorParams;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::embedding::{
    EmbeddingRecord, EmbeddingResponse, EmbeddingUsage, EmbeddingVector,
};
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::passthrough::transformation::types::{PassthroughLogging, RelayCallType};
use http::HeaderMap;
use secrecy::SecretString;
use serde::de::DeserializeOwned;
use serde_json::Number;

pub(super) fn parse<T: DeserializeOwned>(text: &str) -> T {
    serde_json::from_str(text).unwrap()
}

// B249's post_call only observes, so the fixture records what the reply handed it
#[derive(Default)]
pub(super) struct RecordingImageLogging {
    pub(super) inputs: Vec<String>,
}

impl ImageLogging for RecordingImageLogging {
    fn post_call(&mut self, call: PostCall<'_>) {
        self.inputs.push(call.input.to_owned());
    }
}

#[derive(Default)]
pub(super) struct RecordingPassthrough {
    pub(super) call_types: Vec<RelayCallType>,
    pub(super) ocr_call_type_set: bool,
}

impl PassthroughLogging for RecordingPassthrough {
    fn set_call_type(&mut self, call_type: RelayCallType) {
        self.call_types.push(call_type);
    }
}

impl FoundryPassthroughLogging for RecordingPassthrough {
    fn set_ocr_call_type(&mut self) {
        self.ocr_call_type_set = true;
    }
}

pub(super) fn passthrough_plan(litellm_params: Object) -> FoundryPassthroughPlan {
    FoundryPassthroughPlan {
        global_api_base: None,
        global_api_key: None,
        azure: AzurePassthroughConfig {
            api_base_env: Some(String::from("https://res.test")),
            api_key_env: None,
            model_call_details: None,
            chat_reply: NoChatReplyProjection,
            chat_stream: NoChatStreamAssembly,
        },
        ocr: None,
        litellm_params,
    }
}

pub(super) struct NoSources;

fn unavailable() -> SourceFailure {
    SourceFailure {
        class: String::from("ValueError"),
        status: None,
        message: String::from("the fixture resolves no token source"),
    }
}

impl CredentialSources for NoSources {
    fn client_secret<'source>(
        &'source self,
        _record: &ClientSecretRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(unavailable())
    }

    fn username_password<'source>(
        &'source self,
        _record: &UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(unavailable())
    }

    fn sdk_refresh<'source>(
        &'source self,
        _scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(unavailable())
    }

    fn default_credential<'source>(
        &'source self,
        _scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>> {
        None
    }

    fn oidc(&self, _input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        Err(unavailable())
    }
}

pub(super) fn credentials() -> DeploymentSettings {
    DeploymentSettings {
        azure_ad_token: None,
        tenant_id: None,
        client_id: None,
        client_secret: None,
        azure_scope: String::from("https://cognitiveservices.azure.com/.default"),
        azure_username: None,
        azure_password: None,
    }
}

pub(super) struct CountTransport {
    pub(super) status: u16,
    pub(super) body: Vec<u8>,
}

impl CountTokensTransport for CountTransport {
    fn send(
        &self,
        _request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        ready(Ok(UpstreamResponse {
            status: self.status,
            headers: HeaderMap::new(),
            body: self.body.clone(),
        }))
    }
}

pub(super) struct FixtureEmbedder {
    pub(super) vector: Vec<f64>,
}

#[async_trait]
impl QueryEmbeddingExecutor for FixtureEmbedder {
    fn embed(
        &self,
        _model: &str,
        _query: &str,
        _configuration: &VectorParams,
    ) -> Result<EmbeddingResponse, EmbeddingCallError> {
        let numbers = self
            .vector
            .iter()
            .map(|number| Number::from_f64(*number).unwrap())
            .collect();
        Ok(EmbeddingResponse {
            object: String::from("list"),
            data: vec![EmbeddingRecord {
                object: String::from("embedding"),
                embedding: EmbeddingVector::Floats(numbers),
                index: 0,
            }],
            model: String::from("fixture-embedding"),
            usage: EmbeddingUsage {
                prompt_tokens: 1,
                completion_tokens: 0,
                completion_tokens_details: None,
                prompt_tokens_details: None,
                total_tokens: 1,
            },
        })
    }

    async fn aembed(
        &self,
        model: &str,
        query: &str,
        configuration: &VectorParams,
    ) -> Result<EmbeddingResponse, EmbeddingCallError> {
        self.embed(model, query, configuration)
    }
}
