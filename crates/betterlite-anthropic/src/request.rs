pub mod beta_headers;
pub mod code_execution;
mod convert;
pub mod error;
pub mod metadata;
mod preparation;
pub mod system;
mod tests;
mod tool_list;

use std::collections::BTreeMap;

use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use betterlite_prompts::anthropic::assembly::PromptDeps;
use betterlite_prompts::anthropic_results::tool_result::FileBlocks;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use indexmap::IndexMap;
use serde::Deserialize;

use crate::anthropic_capability_catalog::CatalogSources;
use crate::headers::AnthropicHeaders;
use crate::option_primitives::OptionDefaults;

use error::RequestError;
use preparation::Preparation;

// One chat request's inputs; prepare borrows them and leaves every one unchanged.
pub struct AnthropicRequest<'request> {
    pub model: &'request str,
    pub custom_llm_provider: Option<&'request str>,
    pub messages: Vec<Json>,
    pub optional_params: IndexMap<String, Json>,
    pub litellm_params: IndexMap<String, Json>,
    pub headers: AnthropicHeaders,
    pub defaults: &'request OptionDefaults,
    pub catalog: &'request ModelCatalog,
    pub default_chat_max_tokens: i64,
    pub billing_header: BillingHeader,
    pub modify_params: bool,
    pub global_drop_params: bool,
}

// Whether the provider class drops the x-anthropic-billing-header system text, as should_strip_billing_metadata answers
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(from = "bool")]
pub enum BillingHeader {
    #[default]
    Keep,
    Strip,
}

impl From<bool> for BillingHeader {
    fn from(strips: bool) -> Self {
        if strips { Self::Strip } else { Self::Keep }
    }
}

pub struct PreparedRequest {
    pub body: IndexMap<String, Json>,
    pub headers: AnthropicHeaders,
    // Empty when no tool name was rewritten; a caller stores it under the reverse-map key only when non-empty
    pub tool_name_reverse: BTreeMap<String, String>,
}

impl<'request> AnthropicRequest<'request> {
    pub async fn prepare<Source, Transport, Fetcher, Blocks, Ids>(
        &self,
        environment: &mut CapabilityEnvironment<'_, Source, Transport>,
        catalogs: &CatalogSources<'_>,
        prompts: &PromptDeps<'_, Fetcher, Blocks, Ids>,
    ) -> Result<PreparedRequest, RequestError<Blocks::Failure>>
    where
        Source: DynamicMetadataSource,
        Transport: ModelInfoClient,
        Fetcher: RemoteMediaFetcher + Sync,
        Blocks: FileBlocks + Sync,
        Ids: Fn() -> String + Sync,
    {
        let mut preparation: Preparation<'request, Blocks::Failure> =
            Preparation::new(self, self.messages.clone())?;
        preparation.inline_remote_media(prompts.fetcher).await?;
        preparation.run(environment, catalogs, prompts).await?;
        preparation.finish(environment, catalogs).await
    }
}
