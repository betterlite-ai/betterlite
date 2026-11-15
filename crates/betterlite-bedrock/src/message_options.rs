use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_config::sections::anthropic_capability_catalog::Settings;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;

use crate::bedrock_model_capabilities::CapabilityError;

pub mod cache_ttl;
pub mod effort;
pub mod thinking;

mod tests;

pub const ADAPTIVE_THINKING: &str = "supports_adaptive_thinking";
pub const BEDROCK_PROVIDER: &str = "bedrock";
pub const CLEAR_THINKING_EDIT: &str = "clear_thinking_20251015";
pub const CLEAR_THINKING_LOG_PREFIX: &str = "Bedrock clear_thinking_20251015:";

#[derive(Debug, thiserror::Error)]
pub enum MessageOptionsError {
    #[error("thinking.budget_tokens must be an integer")]
    BudgetTokens,
    #[error("{field} is not iterable")]
    NotIterable { field: &'static str },
    #[error(transparent)]
    Capability(#[from] CapabilityError),
}

// The capability and budget inputs the Bedrock messages options read, injected by the caller.
pub struct OptionsSources<'context, 'environment, Source, Transport>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    pub environment: &'context mut CapabilityEnvironment<'environment, Source, Transport>,
    pub catalogs: &'context CatalogSources<'environment>,
    pub budgets: &'context Settings,
}
