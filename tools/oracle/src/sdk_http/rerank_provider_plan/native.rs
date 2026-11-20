use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::api::rerank_provider_plan::loads_rerank_settings;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use betterlite_core::api::rerank_provider_plan::call::ReturnDocuments as CoreReturnDocuments;
use betterlite_core::api::rerank_provider_plan::dispatch::{
    OptionsRequest, Params, RerankOperations, SendRequest,
};
use betterlite_core::api::rerank_provider_plan::plan::{CatalogIdentity, RerankPlan, rerank};
use betterlite_core::api::rerank_provider_plan::selection::SelectedConfig;
use betterlite_core::embedding_rerank_providers::RerankConfig;
use betterlite_error::dispatch::RawException;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::catalog::BundledCatalog;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::api::rerank_contracts::records::RerankResponse;
use betterlite_provider::api::rerank_contracts::{
    OptionalRerankParams, ReturnDocuments as ProviderReturnDocuments,
};
use betterlite_provider::nvidia_nim_rerank::{NvidiaNimRerankSelection, select_rerank_config};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use tracing::subscriber::set_default;
use tracing_subscriber::registry;

use super::super::error::SdkHttpError;
use super::super::result::kinds::SdkResult;
use super::{Call, Outcome};
use input::{core_call, settings_document};
use providers::{Direct, HttpState, direct, family, map_with, not_wired, send_with};
use report::{Diagnostics, failure, logging_state};

mod input;
mod providers;
mod report;

const MOCK: &str = "{mock}";
const FIXTURE_INIT: &str = r#"{"model":"rerank-test","call_type":"rerank","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
const FIXTURE_NOW: &str = "2026-01-01T00:00:05+00:00";

pub(super) async fn execute(
    call: &Call,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let environment: BTreeMap<String, String> = env
        .iter()
        .map(|(name, value)| (name.clone(), value.replace(MOCK, api_base)))
        .collect();
    let document = settings_document(call, api_base)?;
    let settings = loads_rerank_settings(&document, &environment)
        .map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)))?;
    let loaded = ConfigLoader {
        lookup: environment.clone(),
    }
    .load(&document)
    .map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)))?;
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(SdkHttpError::Redactor)?;
    let now = DateTime::parse_from_rfc3339(FIXTURE_NOW)
        .map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
    let operations = Operations {
        environment: &environment,
        config: &loaded,
        source: direct(&environment)?,
        redactor,
        now,
    };
    let catalog = BundledCatalog::default().get();
    let known_model_names = BTreeSet::new();
    let custom_providers = BTreeSet::new();
    let resolution = ResolutionContext {
        catalog: &catalog,
        known_model_names: &known_model_names,
        custom_providers: &custom_providers,
        environment: &environment,
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let identity = CatalogIdentity {
        context: &resolution,
    };
    let plan = RerankPlan {
        identity: &identity,
        operations: &operations,
        settings: &settings,
        environment: &environment,
        redactor: &operations.redactor,
    };
    let mut state = logging_state(&operations.redactor)?;
    let diagnostics = Diagnostics::default();
    let subscription = set_default(registry().with(diagnostics.clone()));
    let outcome = rerank(&plan, core_call(call, api_base)?, &mut state).await;
    drop(subscription);
    let diagnostics = diagnostics.take();
    let value = match outcome {
        Ok(response) => Outcome {
            result: Some(serde_json::to_value(&response).map_err(SdkHttpError::Serialize)?),
            failure: None,
            diagnostics,
        },
        Err(exception) => Outcome {
            result: None,
            failure: Some(failure(&exception)?),
            diagnostics,
        },
    };
    Ok(SdkResult::RerankProviderPlan {
        value: Box::new(value),
    })
}

struct Operations<'call> {
    environment: &'call BTreeMap<String, String>,
    config: &'call Config,
    source: Direct,
    redactor: Redactor,
    now: DateTime<FixedOffset>,
}

impl RerankOperations for Operations<'_> {
    fn map_options(&self, request: OptionsRequest<'_>) -> Result<Params, RawException> {
        let provider = request.custom_llm_provider.unwrap_or_default();
        let return_documents = match request.return_documents {
            CoreReturnDocuments::Default => ProviderReturnDocuments::Default,
            CoreReturnDocuments::Null => ProviderReturnDocuments::Null,
            CoreReturnDocuments::Value(value) => ProviderReturnDocuments::Value(value),
        };
        let params = OptionalRerankParams {
            model: request.model,
            drop_params: request.drop_params,
            query: request.query,
            documents: request.documents,
            custom_llm_provider: request.custom_llm_provider,
            top_n: request.top_n,
            rank_fields: request.rank_fields,
            return_documents,
            max_chunks_per_doc: request.max_chunks_per_doc,
            max_tokens_per_doc: request.max_tokens_per_doc,
            instruction: request.instruction,
            non_default_params: Some(request.non_default_params),
        };
        match request.config {
            SelectedConfig::Cohere => map_with(
                &family(RerankConfig::Cohere, self.config, self.environment),
                params,
                request.model,
                provider,
            ),
            SelectedConfig::CohereV2 => map_with(
                &family(RerankConfig::CohereV2, self.config, self.environment),
                params,
                request.model,
                provider,
            ),
            SelectedConfig::Infinity => map_with(
                &family(RerankConfig::Infinity, self.config, self.environment),
                params,
                request.model,
                provider,
            ),
            SelectedConfig::JinaAi => map_with(
                &family(RerankConfig::JinaAi, self.config, self.environment),
                params,
                request.model,
                provider,
            ),
            SelectedConfig::Voyage => map_with(
                &family(RerankConfig::Voyage, self.config, self.environment),
                params,
                request.model,
                provider,
            ),
            SelectedConfig::NvidiaRetrieval | SelectedConfig::NvidiaRanking => {
                match select_rerank_config(request.model) {
                    NvidiaNimRerankSelection::Retrieval(config) => {
                        map_with(&config, params, request.model, provider)
                    }
                    NvidiaNimRerankSelection::Ranking(config) => {
                        map_with(&config, params, request.model, provider)
                    }
                }
            }
            SelectedConfig::AzureAi
            | SelectedConfig::HostedVllm
            | SelectedConfig::HuggingFace
            | SelectedConfig::Deepinfra
            | SelectedConfig::VertexAi
            | SelectedConfig::FireworksAi
            | SelectedConfig::Watsonx
            | SelectedConfig::Dashscope
            | SelectedConfig::Qwencloud
            | SelectedConfig::QwenAiPlatform => Err(not_wired(request.model, provider)),
        }
    }

    async fn send(
        &self,
        request: SendRequest<'_>,
        logging: &mut LoggingState,
    ) -> Result<RerankResponse, RawException> {
        let http = HttpState {
            source: &self.source,
            redactor: &self.redactor,
            now: self.now,
        };
        match request.config {
            SelectedConfig::Cohere => {
                send_with(
                    &family(RerankConfig::Cohere, self.config, self.environment),
                    request,
                    logging,
                    &http,
                )
                .await
            }
            SelectedConfig::CohereV2 => {
                send_with(
                    &family(RerankConfig::CohereV2, self.config, self.environment),
                    request,
                    logging,
                    &http,
                )
                .await
            }
            SelectedConfig::Infinity => {
                send_with(
                    &family(RerankConfig::Infinity, self.config, self.environment),
                    request,
                    logging,
                    &http,
                )
                .await
            }
            SelectedConfig::JinaAi => {
                send_with(
                    &family(RerankConfig::JinaAi, self.config, self.environment),
                    request,
                    logging,
                    &http,
                )
                .await
            }
            SelectedConfig::Voyage => {
                send_with(
                    &family(RerankConfig::Voyage, self.config, self.environment),
                    request,
                    logging,
                    &http,
                )
                .await
            }
            SelectedConfig::NvidiaRetrieval | SelectedConfig::NvidiaRanking => {
                match select_rerank_config(request.model) {
                    NvidiaNimRerankSelection::Retrieval(config) => {
                        send_with(&config, request, logging, &http).await
                    }
                    NvidiaNimRerankSelection::Ranking(config) => {
                        send_with(&config, request, logging, &http).await
                    }
                }
            }
            SelectedConfig::AzureAi
            | SelectedConfig::HostedVllm
            | SelectedConfig::HuggingFace
            | SelectedConfig::Deepinfra
            | SelectedConfig::VertexAi
            | SelectedConfig::FireworksAi
            | SelectedConfig::Watsonx
            | SelectedConfig::Dashscope
            | SelectedConfig::Qwencloud
            | SelectedConfig::QwenAiPlatform => Err(not_wired(request.model, request.provider)),
        }
    }
}
