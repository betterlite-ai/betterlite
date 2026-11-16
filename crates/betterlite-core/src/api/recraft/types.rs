use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageParams;
use serde::Serialize;

// LiteLLM's RecraftImageGenerationRequestParams is a total=False TypedDict, so a supplied option keeps its key and value as given
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecraftImageGenerationRequest {
    pub prompt: String,
    pub model: String,
    #[serde(flatten)]
    pub options: ImageParams,
}

// LiteLLM builds RecraftImageEditRequestParams as {model, strength, **options} and assigns prompt afterwards
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecraftImageEditRequest {
    pub model: Json,
    pub strength: Json,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<Json>,
    #[serde(flatten)]
    pub options: ImageParams,
}
