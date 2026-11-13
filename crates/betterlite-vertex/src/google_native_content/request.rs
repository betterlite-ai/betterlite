use betterlite_config::sections::vertex_schema_shaping::Settings;
use betterlite_prompts::arguments::json::{Json, Object};
use serde::Serialize;
use serde_with::skip_serializing_none;

use super::capabilities::NativeProvider;
use super::error::NativeContentError;
use super::map::normalize_response_schema;

// the native arguments a caller supplies; contents and tools stay the open JSON the GenAI API takes
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerateContentRequest {
    pub model: String,
    pub contents: Json,
    pub tools: Option<Json>,
    pub generation_config: Object,
    pub system_instruction: Option<Json>,
}

// GoogleGenAIConfig.transform_generate_content_request keeps tools and generationConfig even when empty
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StudioBody {
    pub model: String,
    pub contents: Json,
    pub tools: Json,
    pub generation_config: Object,
    pub system_instruction: Option<Json>,
}

// VertexAIGoogleGenAIConfig.transform_generate_content_request adds a field only when Python calls it truthy
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VertexBody {
    pub model: String,
    pub contents: Json,
    pub tools: Option<Json>,
    pub system_instruction: Option<Json>,
    pub generation_config: Option<Object>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum GenerateContentBody {
    Studio(StudioBody),
    Vertex(VertexBody),
}

pub fn transform_request(
    provider: NativeProvider,
    request: &GenerateContentRequest,
    settings: &Settings,
) -> Result<GenerateContentBody, NativeContentError> {
    let mut config = request.generation_config.clone();
    match provider {
        NativeProvider::Studio => {
            normalize_response_schema(&mut config, &request.model, settings)?;
            Ok(GenerateContentBody::Studio(StudioBody {
                model: request.model.clone(),
                contents: request.contents.clone(),
                tools: request.tools.clone().unwrap_or(Json::Null),
                generation_config: config,
                system_instruction: request
                    .system_instruction
                    .clone()
                    .filter(|instruction| !matches!(instruction, Json::Null)),
            }))
        }
        NativeProvider::Vertex => {
            if !config.is_empty() {
                normalize_response_schema(&mut config, &request.model, settings)?;
            }
            Ok(GenerateContentBody::Vertex(VertexBody {
                model: request.model.clone(),
                contents: request.contents.clone(),
                tools: request.tools.clone().filter(Json::is_truthy),
                system_instruction: request.system_instruction.clone().filter(Json::is_truthy),
                generation_config: (!config.is_empty()).then_some(config),
            }))
        }
    }
}
