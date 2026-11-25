use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::messages_reasoning_route::seams::{
    ModelMetadata, ProviderLookup, ProviderResolution, ResolvedTarget,
};
use betterlite_core::api::messages_response_call::seams::{
    MessagesResponseCallSeams, ResponsesIncludeSupport,
};
use betterlite_models::decisions::Json;
use betterlite_models::full_model_metadata::record::ModelInfo;
use betterlite_models::model_metadata_resolution::names::ModelCostMap;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use indexmap::IndexMap;

// The three lookups the handler asks: the provider's Responses config, get_llm_provider and get_model_info.
pub(super) struct Scripts {
    include: ScriptedInclude,
    resolution: ScriptedResolution,
    metadata: ScriptedMetadata,
}

impl Scripts {
    pub(super) fn new(
        supported: bool,
        provider: Option<String>,
        info: Option<IndexMap<String, Json>>,
    ) -> Result<Self, Box<HelperError>> {
        Ok(Self {
            include: ScriptedInclude { supported },
            resolution: ScriptedResolution { provider },
            metadata: ScriptedMetadata {
                info,
                cost: ModelCostMap::parse(b"{}").or_value_error()?,
            },
        })
    }

    pub(super) fn seams(&self) -> MessagesResponseCallSeams<'_> {
        MessagesResponseCallSeams {
            include: &self.include,
            provider: &self.resolution,
            metadata: &self.metadata,
            redactor: None,
            limits: ApiLimits::default(),
        }
    }
}

struct ScriptedInclude {
    supported: bool,
}

impl ResponsesIncludeSupport for ScriptedInclude {
    fn supports_include(&self, _provider: &str, _model: &str) -> bool {
        self.supported
    }
}

struct ScriptedResolution {
    provider: Option<String>,
}

impl ProviderResolution for ScriptedResolution {
    fn resolve(&self, lookup: &ProviderLookup<'_>) -> Option<ResolvedTarget> {
        self.provider.as_ref().map(|provider| ResolvedTarget {
            model: lookup.model.unwrap_or_default().to_owned(),
            provider: provider.clone(),
            api_base: None,
        })
    }
}

struct ScriptedMetadata {
    info: Option<IndexMap<String, Json>>,
    cost: ModelCostMap,
}

impl ModelMetadata for ScriptedMetadata {
    fn model_info(
        &self,
        _model: Option<&str>,
        _custom_llm_provider: Option<&str>,
    ) -> Option<ModelInfo> {
        self.info.clone().map(|fields| ModelInfo { fields })
    }

    fn model_cost(&self) -> &ModelCostMap {
        &self.cost
    }
}
