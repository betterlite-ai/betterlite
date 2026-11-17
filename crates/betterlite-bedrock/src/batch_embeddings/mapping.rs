use betterlite_prompts::arguments::json::Object;

use super::error::BatchEmbeddingError;
use super::input::coerce_input_to_string;
use super::titan::is_titan_v2_embed_model;
use crate::bedrock_model_capabilities::ModelInfoLookup;
use crate::titan_embeddings::v2;

// An OpenAI /v1/embeddings body as the Titan v2 InvokeModel modelInput; every other embedding model is refused first
pub async fn map_embedding_to_bedrock<Lookup>(
    openai_request_body: &Object,
    model: &str,
    lookup: &Lookup,
) -> Result<Object, BatchEmbeddingError>
where
    Lookup: ModelInfoLookup,
{
    if !is_titan_v2_embed_model(model, lookup).await {
        return Err(BatchEmbeddingError::UnsupportedModel {
            model: model.to_owned(),
        });
    }
    let input_text = coerce_input_to_string(openai_request_body.get("input"), model)?;
    // only model and input leave the body; map_openai_params reads dimensions and encoding_format from the rest
    let non_default: Object = openai_request_body
        .iter()
        .filter(|(name, _)| !matches!(name.as_str(), "model" | "input"))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    let inference_params = v2::map_openai_params(&non_default, Object::new());
    Ok(v2::transform_request(&input_text, &inference_params)?)
}
