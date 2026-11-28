use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::error::ProviderDefaultsError;
use betterlite_models::provider_defaults_a::policies::{
    MantleEndpoint, ModelSets, ProviderPolicies,
};
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::{
    ResolutionContext, ResolutionError, ResolutionRequest, ResolvedModel, resolve,
};
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::passthrough::transformation::types::{
    PassthroughSigning, PassthroughUrlRequest,
};
use url::Url;

use super::PassthroughCallArgs;
use super::PassthroughCallError;
use super::config::ConfigError;

// Which of LiteLLM's two AWS pass-through configs the call is for
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AwsFlavor {
    Bedrock,
    Mantle,
}

// The AWS half of ProviderConfigManager's Bedrock and Bedrock Mantle configs, implemented above core by betterlite-bedrock
#[async_trait]
pub trait AwsPassthrough: Send + Sync {
    fn mantle_endpoint(
        &self,
        request: &ResolutionRequest<'_>,
    ) -> Result<MantleEndpoint, ConfigError>;

    fn complete_url(
        &self,
        flavor: AwsFlavor,
        request: &PassthroughUrlRequest<'_>,
    ) -> Result<(Url, String), ConfigError>;

    async fn sign_request(
        &self,
        flavor: AwsFlavor,
        headers: Headers,
        context: &PassthroughSigning<'_>,
    ) -> Result<(Headers, Option<Vec<u8>>), ConfigError>;
}

// get_llm_provider: the first resolve carries no Mantle endpoint, and only that missing endpoint asks the port
pub(crate) fn resolve_provider(
    args: &PassthroughCallArgs,
    environment: &BTreeMap<String, String>,
    aws: &Arc<dyn AwsPassthrough>,
) -> Result<ResolvedModel, PassthroughCallError> {
    match resolve_with(args, environment, None) {
        Ok(resolved) => Ok(resolved),
        Err(ResolutionError::DefaultsA { source })
            if matches!(&source, ProviderDefaultsError::MissingMantleEndpoint) =>
        {
            let endpoint = aws
                .mantle_endpoint(&resolution_request(args))
                .map_err(PassthroughCallError::MantleEndpoint)?;
            resolve_with(args, environment, Some(endpoint))
                .map_err(PassthroughCallError::Resolution)
        }
        Err(error) => Err(PassthroughCallError::Resolution(error)),
    }
}

fn resolution_request(args: &PassthroughCallArgs) -> ResolutionRequest<'_> {
    ResolutionRequest {
        model: &args.model,
        custom_llm_provider: args.custom_llm_provider.as_deref(),
        api_base: args.api_base.as_deref(),
        api_key: args.api_key.as_ref(),
    }
}

fn resolve_with(
    args: &PassthroughCallArgs,
    environment: &BTreeMap<String, String>,
    mantle: Option<MantleEndpoint>,
) -> Result<ResolvedModel, ResolutionError> {
    let catalog = ModelCatalog::default();
    let known_model_names = BTreeSet::new();
    let custom_providers = BTreeSet::new();
    let policies = ProviderPolicies {
        models: ModelSets::default(),
        mantle,
    };
    resolve(
        &resolution_request(args),
        &ResolutionContext {
            catalog: &catalog,
            known_model_names: &known_model_names,
            custom_providers: &custom_providers,
            environment,
            registry: JSONProviderRegistry::default(),
            policies,
            globals: GlobalCredentials::default(),
        },
    )
}
