use super::super::error::SdkHttpError;
use super::super::result::kinds::SdkResult;
use super::report::outcome;
use super::support::{
    Direct, HttpState, args, direct, family, logging_state, map_with, not_wired, send_with,
};
use super::{Call, Outcome};
use betterlite_config::api::rerank_provider_plan::RerankSettings;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use betterlite_core::api::rerank_call::{AsyncRerankPlan, PlanEntry, arerank};
use betterlite_core::api::rerank_provider_plan::call::ReturnDocuments as CoreReturnDocuments;
use betterlite_core::api::rerank_provider_plan::dispatch::{
    OptionsRequest, Params, RerankOperations, SendRequest,
};
use betterlite_core::api::rerank_provider_plan::plan::{CatalogIdentity, RerankPlan};
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
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;

const FIXTURE_NOW: &str = "2026-01-01T00:00:05+00:00";

pub(super) async fn execute(
    call: &Call,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let environment = substituted_environment(env, api_base);
    let loaded = ConfigLoader {
        lookup: environment.clone(),
    }
    .load("{}")
    .map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)))?;
    let operations = build_operations(&environment, &loaded)?;
    let settings = RerankSettings::default();
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
    let plan = build_plan(&identity, &operations, &settings, &environment);
    let entry = PlanEntry { plan: &plan };
    let facade = build_facade(&identity, &entry, &operations);
    let mut runs = Vec::with_capacity(call.runs.len());
    for run in &call.runs {
        let mut state = logging_state(&operations.redactor)?;
        let result = arerank(&facade, args(run, api_base)?, &mut state).await;
        runs.push(outcome(result)?);
    }
    Ok(SdkResult::RerankCall {
        value: Box::new(Outcome { runs }),
    })
}

fn build_plan<'borrow, Identity>(
    identity: &'borrow Identity,
    operations: &'borrow Operations<'borrow>,
    settings: &'borrow RerankSettings,
    environment: &'borrow BTreeMap<String, String>,
) -> RerankPlan<'borrow, Identity, Operations<'borrow>> {
    RerankPlan {
        identity,
        operations,
        settings,
        environment,
        redactor: &operations.redactor,
    }
}

const fn build_facade<'borrow, Identity, Entry>(
    identity: &'borrow Identity,
    entry: &'borrow Entry,
    operations: &'borrow Operations<'borrow>,
) -> AsyncRerankPlan<'borrow, Identity, Entry> {
    AsyncRerankPlan {
        identity,
        entry,
        redactor: &operations.redactor,
    }
}

fn substituted_environment(
    env: &BTreeMap<String, String>,
    api_base: &str,
) -> BTreeMap<String, String> {
    env.iter()
        .map(|(name, value)| (name.clone(), value.replace(super::MOCK, api_base)))
        .collect()
}

fn build_operations<'settings>(
    environment: &'settings BTreeMap<String, String>,
    config: &'settings Config,
) -> Result<Operations<'settings>, SdkHttpError> {
    Ok(Operations {
        environment,
        config,
        source: direct(environment)?,
        redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(SdkHttpError::Redactor)?,
        now: DateTime::parse_from_rfc3339(FIXTURE_NOW)
            .map_err(|error| SdkHttpError::Invalid(error.to_string()))?,
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
    fn map_options(&self, request: OptionsRequest<'_>) -> Result<Params, Box<RawException>> {
        let (params, config, model, provider) = optional_params(request);
        match family_choice(config) {
            FamilyChoice::Family(kind) => map_with(
                &family(kind, self.config, self.environment),
                params,
                model,
                provider,
            ),
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
        let http = HttpState {
            source: &self.source,
            redactor: &self.redactor,
            now: self.now,
        };
        match family_choice(request.config) {
            FamilyChoice::Family(kind) => {
                let config = family(kind, self.config, self.environment);
                send_with(&config, request, logging, &http).await
            }
            FamilyChoice::Unsupported => Err(Box::new(not_wired(request.model, request.provider))),
        }
    }
}

// The two shapes a selected config takes here: a provider family config, or a handler this adapter does not wire
enum FamilyChoice {
    Family(RerankConfig),
    Unsupported,
}

const fn family_choice(config: SelectedConfig) -> FamilyChoice {
    match config {
        SelectedConfig::Cohere => FamilyChoice::Family(RerankConfig::Cohere),
        SelectedConfig::CohereV2 => FamilyChoice::Family(RerankConfig::CohereV2),
        SelectedConfig::Infinity => FamilyChoice::Family(RerankConfig::Infinity),
        SelectedConfig::JinaAi => FamilyChoice::Family(RerankConfig::JinaAi),
        SelectedConfig::Voyage => FamilyChoice::Family(RerankConfig::Voyage),
        SelectedConfig::NvidiaRetrieval
        | SelectedConfig::NvidiaRanking
        | SelectedConfig::AzureAi
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
