use betterlite_bedrock::aws_endpoints::provider::{
    BedrockInvokeProvider as SourceProvider, invoke_provider,
};
use betterlite_bedrock::model_routes::{BedrockRoute as SourceRoute, ModelRouteSelector, ParamMap};
use betterlite_context::bedrock_config_selection::chat::{BedrockInvokeProvider, BedrockRoute};
use betterlite_context::chat_config_selection::facts::{BedrockFacts, ModelFamilyProbe};
use betterlite_context::chat_config_selection::json_provider::{
    JsonProviderLookup, JsonProviderProjection,
};
use betterlite_context::chat_config_selection::selection::ChatConfigRequest;
use betterlite_context::chat_config_selection::vertex::VertexModelLists;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::openai_base_config::{
    CatalogCapabilities, ModelFamily, azure_model_family, is_o_series_model, model_family,
};
use indexmap::IndexSet;
use serde::Deserialize;

use super::{Call, Catalog, JsonProviderEntry, invalid};
use crate::helpers::HelperError;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(crate) struct VertexLists {
    mistral_models: Vec<String>,
    ai21_models: Vec<String>,
}

struct Families {
    catalog: CatalogCapabilities,
}

impl ModelFamilyProbe for Families {
    fn openai_o_series(&self, model: &str) -> bool {
        is_o_series_model(model, &self.catalog)
    }

    fn openai_gpt_5(&self, model: &str) -> bool {
        matches!(
            model_family(model, &self.catalog),
            ModelFamily::GptReasoning
        )
    }

    fn openai_catalog_model(&self, model: &str) -> bool {
        self.catalog.is_catalog_model(model)
    }

    fn azure_gpt_5(&self, model: &str) -> bool {
        matches!(azure_model_family(model), ModelFamily::GptReasoning)
    }
}

struct JsonProviders<'entry> {
    entries: &'entry [JsonProviderEntry],
}

impl JsonProviderProjection for JsonProviders<'_> {
    fn lookup(&self, provider: ProviderId) -> JsonProviderLookup {
        let Some(entry) = self.entries.iter().find(|entry| entry.provider == provider) else {
            return JsonProviderLookup::Absent;
        };
        match (entry.exists, entry.record) {
            (false, _) => JsonProviderLookup::Absent,
            (true, true) => JsonProviderLookup::Record,
            (true, false) => JsonProviderLookup::MissingRecord,
        }
    }
}

pub(super) struct Fixtures<'input> {
    families: Families,
    vertex: VertexModelLists<'input>,
    json: JsonProviders<'input>,
}

impl<'input> Fixtures<'input> {
    pub(super) fn new(
        catalog: &Catalog,
        vertex: &'input VertexLists,
        providers: &'input [JsonProviderEntry],
    ) -> Self {
        Self {
            families: Families {
                catalog: CatalogCapabilities {
                    chat_models: catalog.chat_models.iter().cloned().collect(),
                    text_models: catalog.text_models.iter().cloned().collect(),
                },
            },
            vertex: VertexModelLists {
                mistral_models: &vertex.mistral_models,
                ai21_models: &vertex.ai21_models,
            },
            json: JsonProviders { entries: providers },
        }
    }

    pub(super) fn request<'call>(
        &'call self,
        call: &'call Call,
        bedrock_base_model: &'call str,
    ) -> Result<ChatConfigRequest<'call>, Box<HelperError>> {
        Ok(ChatConfigRequest {
            model: &call.model,
            provider: call.provider,
            base_model: call.base_model.as_deref(),
            families: &self.families,
            bedrock: bedrock_facts(&call.model, bedrock_base_model)?,
            vertex: self.vertex,
            json_providers: &self.json,
        })
    }
}

const fn route_of(route: SourceRoute) -> BedrockRoute {
    match route {
        SourceRoute::Converse => BedrockRoute::Converse,
        SourceRoute::Invoke => BedrockRoute::Invoke,
        SourceRoute::ClaudePlatform => BedrockRoute::ClaudePlatform,
        SourceRoute::ConverseLike => BedrockRoute::ConverseLike,
        SourceRoute::Agent => BedrockRoute::Agent,
        SourceRoute::Agentcore => BedrockRoute::Agentcore,
        SourceRoute::AsyncInvoke => BedrockRoute::AsyncInvoke,
        SourceRoute::Openai => BedrockRoute::Openai,
        SourceRoute::Mantle => BedrockRoute::Mantle,
        SourceRoute::ChatCompletions => BedrockRoute::ChatCompletions,
    }
}

const fn provider_of(provider: SourceProvider) -> BedrockInvokeProvider {
    match provider {
        SourceProvider::Cohere => BedrockInvokeProvider::Cohere,
        SourceProvider::Anthropic => BedrockInvokeProvider::Anthropic,
        SourceProvider::Mistral => BedrockInvokeProvider::Mistral,
        SourceProvider::Amazon => BedrockInvokeProvider::Amazon,
        SourceProvider::Meta => BedrockInvokeProvider::Meta,
        SourceProvider::Llama => BedrockInvokeProvider::Llama,
        SourceProvider::Ai21 => BedrockInvokeProvider::Ai21,
        SourceProvider::Nova => BedrockInvokeProvider::Nova,
        SourceProvider::DeepseekR1 => BedrockInvokeProvider::DeepseekR1,
        SourceProvider::Qwen3 => BedrockInvokeProvider::Qwen3,
        SourceProvider::Qwen2 => BedrockInvokeProvider::Qwen2,
        SourceProvider::Twelvelabs => BedrockInvokeProvider::Twelvelabs,
        SourceProvider::Openai => BedrockInvokeProvider::Openai,
        SourceProvider::Stability => BedrockInvokeProvider::Stability,
        SourceProvider::Moonshot => BedrockInvokeProvider::Moonshot,
    }
}

fn bedrock_facts<'model>(
    model: &str,
    base_model: &'model str,
) -> Result<BedrockFacts<'model>, Box<HelperError>> {
    let price_map = ParamMap::new();
    let converse_models = IndexSet::new();
    let selector = ModelRouteSelector::new(&price_map, &converse_models, false)
        .map_err(|error| invalid(&error.to_string()))?;
    let route = selector
        .route(model, None)
        .map_err(|error| invalid(&error.to_string()))?;
    Ok(BedrockFacts {
        route: route_of(route),
        invoke_provider: invoke_provider(model).map(provider_of),
        base_model,
    })
}
