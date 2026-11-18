use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_anthropic::option_primitives::OptionDefaults;
use betterlite_anthropic::request::error::RequestError;
use betterlite_anthropic::request::{AnthropicRequest, BillingHeader, PreparedRequest};
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use betterlite_prompts::anthropic::assembly::PromptDeps;
use betterlite_prompts::anthropic_results::tool_result::FileBlocks;
use betterlite_prompts::media::inline::RemoteMediaFetcher;

use super::config::Validated;

pub struct Preparation<'deps, Source, Metadata, Fetcher, Blocks, Ids> {
    pub environment: CapabilityEnvironment<'deps, Source, Metadata>,
    pub catalogs: CatalogSources<'deps>,
    pub prompts: PromptDeps<'deps, Fetcher, Blocks, Ids>,
    pub catalog: &'deps ModelCatalog,
    pub defaults: &'deps OptionDefaults,
    pub max_tokens: i64,
    pub billing: BillingHeader,
    pub modify_params: bool,
    pub drop_params: bool,
}

impl<Source, Metadata, Fetcher, Blocks, Ids> Preparation<'_, Source, Metadata, Fetcher, Blocks, Ids>
where
    Source: DynamicMetadataSource,
    Metadata: ModelInfoClient,
    Fetcher: RemoteMediaFetcher + Sync,
    Blocks: FileBlocks + Sync,
    Ids: Fn() -> String + Sync,
{
    pub async fn prepare(
        &mut self,
        model: &str,
        provider: &str,
        input: Validated,
    ) -> Result<PreparedRequest, RequestError<Blocks::Failure>> {
        let request = AnthropicRequest {
            model,
            custom_llm_provider: Some(provider),
            messages: input.messages,
            optional_params: input.optional,
            litellm_params: input.litellm,
            headers: input.headers,
            defaults: self.defaults,
            catalog: self.catalog,
            default_chat_max_tokens: self.max_tokens,
            billing_header: self.billing,
            modify_params: self.modify_params,
            global_drop_params: self.drop_params,
        };
        request
            .prepare(&mut self.environment, &self.catalogs, &self.prompts)
            .await
    }
}
