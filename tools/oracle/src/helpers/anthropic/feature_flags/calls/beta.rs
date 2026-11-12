use std::collections::BTreeSet;

use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_anthropic::feature_flags::beta::{BetaRequest, beta_list};
use betterlite_anthropic::feature_flags::{AnthropicFeatures, Feature, FeatureParams};
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum FeatureName {
    FileId,
    McpServer,
    OutputConfig,
    ThinkingDisplay,
    ToolChange,
}

impl FeatureName {
    const fn feature(self) -> Feature {
        match self {
            Self::FileId => Feature::FileId,
            Self::McpServer => Feature::McpServer,
            Self::OutputConfig => Feature::MidConversationOutputConfig,
            Self::ThinkingDisplay => Feature::ThinkingDisplayUpdates,
            Self::ToolChange => Feature::MidConversationToolChange,
        }
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct Beta {
    model: String,
    custom_llm_provider: String,
    params: FeatureParams,
    computer_tool_used: Option<String>,
    prompt_caching_set: bool,
    features: Vec<FeatureName>,
}

impl Beta {
    pub(crate) async fn answer<Source, Transport>(
        &self,
        environment: &mut CapabilityEnvironment<'_, Source, Transport>,
        catalogs: &CatalogSources<'_>,
    ) -> Value
    where
        Source: DynamicMetadataSource,
        Transport: ModelInfoClient,
    {
        let features = AnthropicFeatures {
            flags: self.flags(),
            computer_tool: self.computer_tool_used.clone(),
        };
        let request = BetaRequest {
            model: &self.model,
            custom_llm_provider: &self.custom_llm_provider,
            params: Some(&self.params),
            features: &features,
            prompt_caching_set: self.prompt_caching_set,
        };
        let betas = beta_list(&request, environment, catalogs).await;
        json!({"betas": betas})
    }

    fn flags(&self) -> BTreeSet<Feature> {
        let mut flags = BTreeSet::new();
        for name in &self.features {
            flags.insert(name.feature());
        }
        flags
    }
}
