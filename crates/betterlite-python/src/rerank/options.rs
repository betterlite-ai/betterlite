use betterlite_core::api::rerank_provider_plan::call::ReturnDocuments as CoreReturnDocuments;
use betterlite_core::api::rerank_provider_plan::dispatch::{OptionsRequest, Params, value_failure};
use betterlite_core::api::rerank_provider_plan::selection::SelectedConfig;
use betterlite_core::embedding_rerank_providers::RerankConfig;
use betterlite_error::dispatch::RawException;
use betterlite_models::decisions::Json;
use betterlite_provider::api::rerank_contracts::{
    BaseRerankConfig, OptionalRerankParams, ReturnDocuments as ProviderReturnDocuments,
    get_optional_rerank_params,
};

// The three shapes a selected config takes: a provider family config, one of Nvidia's two clients, or no handler.
pub(super) enum FamilyChoice {
    Family(RerankConfig),
    Nvidia,
    Refused(FamilyRefusal),
}

// Why no handler runs: LiteLLM refuses the provider itself, or its own handler is not wired here yet.
pub(super) enum FamilyRefusal {
    Unsupported,
    NotWired,
}

// LiteLLM's trailing else refuses the default Cohere config for every provider but cohere and the proxy
#[cfg_attr(test, mutants::skip)]
pub(super) fn family_choice(config: SelectedConfig, provider: &str) -> FamilyChoice {
    if provider == "together_ai" || provider == "bedrock" {
        return FamilyChoice::Refused(FamilyRefusal::NotWired);
    }
    match config {
        SelectedConfig::Cohere | SelectedConfig::CohereV2
            if provider != "cohere" && provider != "litellm_proxy" =>
        {
            FamilyChoice::Refused(FamilyRefusal::Unsupported)
        }
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
        | SelectedConfig::QwenAiPlatform => FamilyChoice::Refused(FamilyRefusal::NotWired),
    }
}

#[cfg_attr(test, mutants::skip)]
pub(super) fn refusal(model: &str, provider: &str, refusal: FamilyRefusal) -> RawException {
    match refusal {
        FamilyRefusal::Unsupported => value_failure(
            &format!("Unsupported provider: {provider}"),
            model,
            provider,
        ),
        FamilyRefusal::NotWired => value_failure(
            &format!("the {provider} rerank handler is not wired into betterlite"),
            model,
            provider,
        ),
    }
}

#[cfg_attr(test, mutants::skip)]
pub(super) fn map_with<C>(
    config: &C,
    request: OptionalRerankParams<'_>,
    model: &str,
    provider: &str,
) -> Result<Params, Box<RawException>>
where
    C: BaseRerankConfig,
{
    get_optional_rerank_params(config, request)
        .map_err(|error| Box::new(value_failure(&error.to_string(), model, provider)))
}

#[cfg_attr(test, mutants::skip)]
pub(super) fn optional_params(
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
    place_rank_fields(non_default_params, rank_fields, config);
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

// LiteLLM's cohere map takes rank_fields as its own argument; the provider map reads the non-default params
#[cfg_attr(test, mutants::skip)]
fn place_rank_fields(
    non_default_params: &mut Params,
    rank_fields: Option<&[String]>,
    config: SelectedConfig,
) {
    let reads_params = matches!(
        config,
        SelectedConfig::Cohere | SelectedConfig::CohereV2 | SelectedConfig::Infinity
    );
    if !reads_params {
        return;
    }
    if let Some(fields) = rank_fields {
        non_default_params.insert(
            String::from("rank_fields"),
            Json::List(
                fields
                    .iter()
                    .map(|field| Json::Text(field.clone()))
                    .collect(),
            ),
        );
    }
}

#[cfg_attr(test, mutants::skip)]
const fn mapped_documents(return_documents: CoreReturnDocuments) -> ProviderReturnDocuments {
    match return_documents {
        CoreReturnDocuments::Default => ProviderReturnDocuments::Default,
        CoreReturnDocuments::Null => ProviderReturnDocuments::Null,
        CoreReturnDocuments::Value(value) => ProviderReturnDocuments::Value(value),
    }
}
