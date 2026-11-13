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
    pub tools: Json,
    pub generation_config: Object,
    pub system_instruction: Json,
}

// Studio keeps tools and generationConfig even when absent or empty, Vertex adds a field only when Python calls it truthy
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateContentBody {
    pub model: String,
    pub contents: Json,
    pub tools: Option<Json>,
    pub system_instruction: Option<Json>,
    pub generation_config: Option<Object>,
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
            Ok(GenerateContentBody {
                model: request.model.clone(),
                contents: request.contents.clone(),
                tools: Some(request.tools.clone()),
                system_instruction: (!matches!(&request.system_instruction, Json::Null))
                    .then(|| request.system_instruction.clone()),
                generation_config: Some(config),
            })
        }
        NativeProvider::Vertex => {
            if !config.is_empty() {
                normalize_response_schema(&mut config, &request.model, settings)?;
            }
            Ok(GenerateContentBody {
                model: request.model.clone(),
                contents: request.contents.clone(),
                tools: request.tools.is_truthy().then(|| request.tools.clone()),
                system_instruction: request
                    .system_instruction
                    .is_truthy()
                    .then(|| request.system_instruction.clone()),
                generation_config: (!config.is_empty()).then_some(config),
            })
        }
    }
}
