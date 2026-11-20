use super::super::providers::{Direct, HttpState, family, map_with, not_wired, send_with};
use betterlite_config::types::Config;
use betterlite_core::api::rerank_provider_plan::call::ReturnDocuments as CoreReturnDocuments;
use betterlite_core::api::rerank_provider_plan::dispatch::{
    OptionsRequest, Params, RerankOperations, SendRequest,
};
use betterlite_core::api::rerank_provider_plan::selection::SelectedConfig;
use betterlite_core::embedding_rerank_providers::RerankConfig;
use betterlite_error::dispatch::RawException;
use betterlite_logging::logging_state::LoggingState;
use betterlite_openai::api::rerank_contracts::records::RerankResponse;
use betterlite_provider::api::rerank_contracts::{
    BaseRerankConfig, OptionalRerankParams, ReturnDocuments as ProviderReturnDocuments,
};
use betterlite_provider::nvidia_nim_rerank::{NvidiaNimRerankSelection, select_rerank_config};
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use std::collections::BTreeMap;
use std::future::Future;
pub(super) struct Operations<'call> {
    pub(super) environment: &'call BTreeMap<String, String>,
    pub(super) config: &'call Config,
    pub(super) source: Direct,
    pub(super) redactor: Redactor,
    pub(super) now: DateTime<FixedOffset>,
}
impl RerankOperations for Operations<'_> {
    fn map_options(&self, request: OptionsRequest<'_>) -> Result<Params, Box<RawException>> {
        let (params, config, model, provider) = optional_params(request);
        match family_choice(config) {
            FamilyChoice::Family(kind) => map_with(
                &family(kind, self.config, self.environment),
                params,
                model,
                provider,
            ),
            FamilyChoice::Nvidia => match select_rerank_config(model) {
                NvidiaNimRerankSelection::Retrieval(selected) => {
                    map_with(&selected, params, model, provider)
                }
                NvidiaNimRerankSelection::Ranking(selected) => {
                    map_with(&selected, params, model, provider)
                }
            },
            FamilyChoice::Unsupported => Err(Box::new(not_wired(model, provider))),
        }
    }
    fn send(
        &self,
        request: SendRequest<'_>,
        logging: &mut LoggingState,
    ) -> impl Future<Output = Result<RerankResponse, Box<RawException>>> + Send {
        self.send_config(request, logging)
    }
}
impl Operations<'_> {
    async fn send_config(
        &self,
        request: SendRequest<'_>,
        logging: &mut LoggingState,
    ) -> Result<RerankResponse, Box<RawException>> {
        match family_choice(request.config) {
            FamilyChoice::Family(kind) => {
                let config = family(kind, self.config, self.environment);
                self.send_family(&config, request, logging).await
            }
            FamilyChoice::Nvidia => match select_rerank_config(request.model) {
                NvidiaNimRerankSelection::Retrieval(selected) => {
                    self.send_family(&selected, request, logging).await
                }
                NvidiaNimRerankSelection::Ranking(selected) => {
                    self.send_family(&selected, request, logging).await
                }
            },
            FamilyChoice::Unsupported => Err(Box::new(not_wired(request.model, request.provider))),
        }
    }
    async fn send_family<C>(
        &self,
        config: &C,
        request: SendRequest<'_>,
        logging: &mut LoggingState,
    ) -> Result<RerankResponse, Box<RawException>>
    where
        C: BaseRerankConfig,
    {
        let http = HttpState {
            source: &self.source,
            redactor: &self.redactor,
            now: self.now,
        };
        send_with(config, request, logging, &http).await
    }
}
// The three shapes a selected config takes: a provider family config, one of Nvidia's two clients, or no handler
enum FamilyChoice {
    Family(RerankConfig),
    Nvidia,
    Unsupported,
}
const fn family_choice(config: SelectedConfig) -> FamilyChoice {
    match config {
        SelectedConfig::Cohere => FamilyChoice::Family(RerankConfig::Cohere),
        SelectedConfig::CohereV2 => FamilyChoice::Family(RerankConfig::CohereV2),
        SelectedConfig::Infinity => FamilyChoice::Family(RerankConfig::Infinity),
        SelectedConfig::JinaAi => FamilyChoice::Family(RerankConfig::JinaAi),
        SelectedConfig::Voyage => FamilyChoice::Family(RerankConfig::Voyage),
        SelectedConfig::NvidiaRetrieval | SelectedConfig::NvidiaRanking => FamilyChoice::Nvidia,
        SelectedConfig::AzureAi
        | SelectedConfig::HostedVllm
        | SelectedConfig::HuggingFace
        | SelectedConfig::Deepinfra
        | SelectedConfig::VertexAi
        | SelectedConfig::FireworksAi
        | SelectedConfig::Watsonx
        | SelectedConfig::Dashscope
        | SelectedConfig::Qwencloud
        | SelectedConfig::QwenAiPlatform => FamilyChoice::Unsupported,
    }
}
fn optional_params(
    request: OptionsRequest<'_>,
) -> (OptionalRerankParams<'_>, SelectedConfig, &str, &str) {
    let OptionsRequest {
        config,
        model,
        drop_params,
        query,
        documents,
        custom_llm_provider,
        top_n,
        rank_fields,
        return_documents,
        max_chunks_per_doc,
        max_tokens_per_doc,
        instruction,
        non_default_params,
    } = request;
    let params = OptionalRerankParams {
        model,
        drop_params,
        query,
        documents,
        custom_llm_provider,
        top_n,
        rank_fields,
        return_documents: mapped_documents(return_documents),
        max_chunks_per_doc,
        max_tokens_per_doc,
        instruction,
        non_default_params: Some(non_default_params),
    };
    (
        params,
        config,
        model,
        custom_llm_provider.unwrap_or_default(),
    )
}
const fn mapped_documents(return_documents: CoreReturnDocuments) -> ProviderReturnDocuments {
    match return_documents {
        CoreReturnDocuments::Default => ProviderReturnDocuments::Default,
        CoreReturnDocuments::Null => ProviderReturnDocuments::Null,
        CoreReturnDocuments::Value(value) => ProviderReturnDocuments::Value(value),
    }
}
