use std::sync::Arc;

use async_trait::async_trait;
use betterlite_models::catalog::BundledCatalog;
use betterlite_models::decisions::Json;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::router_predicates::params::{ParamSupport, ResolvedProvider};
use betterlite_router::static_eligibility::tokens::{TokenCountError, TokenCounter, TokenLimits};
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::messages::{count_extra, count_messages, message_params};
use betterlite_tokenizer::count::request::Message;
use betterlite_tokenizer::count::settings::Settings;
use betterlite_tokenizer::count::text::{Counted, Texter};
use betterlite_tokenizer::engine::Engine;
use betterlite_tokenizer::engine::assets;
use betterlite_tokenizer::engine::error::EngineError;
use betterlite_tokenizer::engine::limits::{
    BlockingCounter, BuildJob, ConvertJob, CountJob, CountingService,
};
use betterlite_tokenizer::engine::tiktoken::{Encoding, TiktokenEngine};

use crate::router_probes::error::OperationError;

struct InlineCounter;

#[async_trait]
impl BlockingCounter for InlineCounter {
    async fn count(&self, job: CountJob) -> Result<usize, EngineError> {
        job()
    }

    async fn build(&self, job: BuildJob) -> Result<Engine, EngineError> {
        job()
    }

    async fn convert(&self, job: ConvertJob) -> Result<Vec<u8>, EngineError> {
        job()
    }
}

// litellm.token_counter with the empty model _count_pre_call_check_tokens passes: cl100k with the bundled vocabulary.
pub(crate) struct ProbeCounter {
    service: CountingService,
    counted: Counted,
    settings: Settings,
    catalog: Arc<betterlite_models::catalog::ModelCatalog>,
}

impl ProbeCounter {
    pub(crate) fn new() -> Result<Self, OperationError> {
        let settings = Settings::default();
        let data = assets::bundled(Encoding::Cl100kBase).ok_or_else(|| {
            OperationError::InvalidRequest {
                message: "the cl100k vocabulary is not bundled".to_owned(),
            }
        })?;
        let engine =
            TiktokenEngine::from_bpe_file(Encoding::Cl100kBase, &data).map_err(|error| {
                OperationError::InvalidRequest {
                    message: error.to_string(),
                }
            })?;
        let limits = settings
            .encode_limits()
            .map_err(|error| OperationError::InvalidRequest {
                message: error.to_string(),
            })?;
        Ok(Self {
            service: CountingService::new(1, Arc::new(InlineCounter), limits),
            counted: Counted {
                engine: Engine::Tiktoken(Arc::new(engine)),
                chunks: Some(settings.chunk_chars),
            },
            settings,
            catalog: BundledCatalog::default().get(),
        })
    }
}

#[async_trait]
impl TokenCounter for ProbeCounter {
    async fn count_messages(
        &self,
        messages: &[Message],
        tools: &[Json],
    ) -> Result<usize, TokenCountError> {
        let texter = Texter {
            counter: &self.service,
            counted: &self.counted,
            max_exact_chars: self.settings.max_exact_chars,
        };
        let params = message_params("", &self.catalog);
        let total = count_messages(&texter, params, messages).await?;
        let includes_system = messages.iter().any(|message| message.role == "system");
        let extra = count_extra(
            &texter,
            Some(tools).filter(|entries| !entries.is_empty()),
            None,
            includes_system,
        )
        .await?;
        let total = total.checked_add(extra).ok_or(CountError::Overflow)?;
        Ok(total)
    }
}

// The probe deployments declare their context window in model_info, which get_router_model_info lets win over the catalog.
pub(crate) struct ProbeLimits;

impl TokenLimits for ProbeLimits {
    fn max_input_tokens(&self, spec: &DeploymentSpec, _model: &str) -> Option<usize> {
        match spec.model_info.get("max_input_tokens") {
            Some(Json::Number(number)) => number
                .as_u64()
                .and_then(|value| usize::try_from(value).ok()),
            Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_))
            | None => None,
        }
    }
}

// get_llm_provider on the probe's provider-prefixed models, and OpenAIGPTConfig.get_supported_openai_params.
pub(crate) struct ProbeSupport;

impl ParamSupport for ProbeSupport {
    fn is_authenticating_provider(&self, _model: &str, _custom_llm_provider: Option<&str>) -> bool {
        false
    }

    fn resolve_provider(
        &self,
        model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<ResolvedProvider> {
        match model.split_once('/') {
            Some((provider, bare)) => Some(ResolvedProvider {
                model: bare.to_owned(),
                custom_llm_provider: provider.to_owned(),
            }),
            None => Some(ResolvedProvider {
                model: model.to_owned(),
                custom_llm_provider: "openai".to_owned(),
            }),
        }
    }

    fn supported_openai_params(
        &self,
        model: &str,
        custom_llm_provider: &str,
        _base_model: Option<&str>,
    ) -> Option<Vec<String>> {
        if custom_llm_provider != "openai" {
            return None;
        }
        // gpt-4 and gpt-3.5-turbo-16k are the two models whose OpenAI config leaves response_format out
        match model {
            "gpt-4" | "gpt-3.5-turbo-16k" => Some(Vec::new()),
            _ => Some(vec!["response_format".to_owned()]),
        }
    }
}
