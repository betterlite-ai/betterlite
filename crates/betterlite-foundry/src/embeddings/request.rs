use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::media::base64_string::is_base64_encoded;
use indexmap::IndexMap;
use serde::Serialize;

// the Cohere options LiteLLM names, plus whatever else the caller forwards to both projections
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct FoundryEmbeddingOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncate: Option<String>,
    #[serde(flatten)]
    pub passthrough: IndexMap<String, Json>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImageEmbeddingInput<'input> {
    pub image: &'input str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImageEmbeddingRequest<'input> {
    pub input: Vec<ImageEmbeddingInput<'input>>,
    #[serde(flatten)]
    pub options: FoundryEmbeddingOptions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundryEmbeddingRequests<'input> {
    pub image: ImageEmbeddingRequest<'input>,
    pub text_input: Vec<&'input str>,
    pub image_indices: Vec<usize>,
}

#[must_use]
pub fn transform_request<'input>(
    input: &'input [String],
    options: &FoundryEmbeddingOptions,
) -> FoundryEmbeddingRequests<'input> {
    let mut image = Vec::new();
    let mut image_indices = Vec::new();
    let mut text_input = Vec::new();
    for (index, text) in input.iter().enumerate() {
        if is_base64_encoded(text) {
            image.push(ImageEmbeddingInput {
                image: text.as_str(),
            });
            image_indices.push(index);
        } else {
            text_input.push(text.as_str());
        }
    }
    FoundryEmbeddingRequests {
        image: ImageEmbeddingRequest {
            input: image,
            options: options.clone(),
        },
        text_input,
        image_indices,
    }
}
