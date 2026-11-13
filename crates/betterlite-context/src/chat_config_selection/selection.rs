use betterlite_models::identity::provider_id::ProviderId;

use crate::bedrock_config_selection::chat::select_bedrock_chat_config;

use super::azure;
use super::cohere;
use super::facts::{BedrockFacts, ModelFamilyProbe};
use super::foundry;
use super::json_provider::{JsonProviderProjection, select_json_provider};
use super::table::{ConfigFactory, PROVIDER_CONFIG_ROWS, ProviderConfigTable};
use super::tags::{ChatConfigTag, ConfigConstruction, ConfigSelection};
use super::vertex::{self, VertexModelLists};

#[derive(Debug, thiserror::Error)]
pub enum ChatConfigError {
    #[error("the JSON provider registry lists {provider} but holds no record for it")]
    MissingJsonRecord { provider: ProviderId },
}

pub struct ChatConfigRequest<'source> {
    pub model: &'source str,
    pub provider: ProviderId,
    pub base_model: Option<&'source str>,
    pub families: &'source dyn ModelFamilyProbe,
    pub bedrock: BedrockFacts<'source>,
    pub vertex: VertexModelLists<'source>,
    pub json_providers: &'source dyn JsonProviderProjection,
}

pub fn select_chat_config(
    request: &ChatConfigRequest<'_>,
    table: &mut ProviderConfigTable,
) -> Result<Option<ConfigSelection<ChatConfigTag>>, ChatConfigError> {
    if request.provider == ProviderId::Openai {
        return select_openai_config(request, table);
    }
    if request.provider == ProviderId::Azure {
        return Ok(Some(select_azure(request)));
    }
    select_from_table(request, table)
}

fn select_openai_config(
    request: &ChatConfigRequest<'_>,
    table: &mut ProviderConfigTable,
) -> Result<Option<ConfigSelection<ChatConfigTag>>, ChatConfigError> {
    if request.families.openai_o_series(request.model) {
        return Ok(Some(ConfigSelection {
            tag: ChatConfigTag::OpenAiOSeries,
            construction: ConfigConstruction::Shared,
        }));
    }
    if request.families.openai_gpt_5(request.model) {
        return Ok(Some(ConfigSelection {
            tag: ChatConfigTag::OpenAiGpt5,
            construction: ConfigConstruction::New(Vec::new()),
        }));
    }
    if !request.families.openai_catalog_model(request.model) {
        return Ok(Some(ConfigSelection {
            tag: ChatConfigTag::OpenAiUnknownModel,
            construction: ConfigConstruction::New(Vec::new()),
        }));
    }
    select_from_table(request, table)
}

fn select_azure(request: &ChatConfigRequest<'_>) -> ConfigSelection<ChatConfigTag> {
    // LiteLLM runs detection on `base_model or model`, so an empty base_model falls back to the model
    let detection_model = request
        .base_model
        .filter(|value| !value.is_empty())
        .unwrap_or(request.model);
    ConfigSelection {
        tag: azure::select_config(detection_model, request.families),
        construction: ConfigConstruction::New(Vec::new()),
    }
}

fn select_from_table(
    request: &ChatConfigRequest<'_>,
    table: &mut ProviderConfigTable,
) -> Result<Option<ConfigSelection<ChatConfigTag>>, ChatConfigError> {
    let rows = table.rows.get_or_insert(&PROVIDER_CONFIG_ROWS);
    if let Some((_, tag)) = rows
        .direct
        .iter()
        .find(|(provider, _)| *provider == request.provider)
    {
        return Ok(Some(ConfigSelection {
            tag: *tag,
            construction: ConfigConstruction::New(Vec::new()),
        }));
    }
    let Some((_, factory)) = rows
        .model_aware
        .iter()
        .find(|(provider, _)| *provider == request.provider)
    else {
        return select_json_provider(request);
    };
    let tag = match factory {
        ConfigFactory::AzureAi => foundry::select_config(request.model),
        ConfigFactory::VertexAi => vertex::select_config(request.model, request.vertex),
        ConfigFactory::Bedrock => ChatConfigTag::Bedrock(select_bedrock_chat_config(
            request.bedrock.route,
            request.bedrock.invoke_provider,
            request.bedrock.base_model,
        )),
        ConfigFactory::Cohere => cohere::select_config(request.model),
    };
    Ok(Some(ConfigSelection {
        tag,
        construction: ConfigConstruction::New(vec![request.model.to_owned()]),
    }))
}
