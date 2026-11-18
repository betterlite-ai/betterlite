use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::media::base64_string::{get_base64_str, is_base64_encoded};

use super::error::TitanEmbeddingsError;
use super::response::{self, TitanEmbeddingResponse, TitanPromptTokensDetails};

pub const SUPPORTED_OPENAI_PARAMS: &[&str] = &["dimensions"];

#[must_use]
pub fn map_openai_params(non_default_params: &Object, mut optional_params: Object) -> Object {
    for (name, value) in non_default_params {
        if name.as_str() != "dimensions" {
            continue;
        }
        let mut config = Object::new();
        config.insert(String::from("outputEmbeddingLength"), value.clone());
        optional_params.insert(String::from("embeddingConfig"), Json::Object(config));
    }
    optional_params
}

// B183 decides image versus text, and every inference key then overwrites or extends the request
#[must_use]
pub fn transform_request(input: &str, inference_params: &Object) -> Object {
    let mut request = Object::new();
    if is_base64_encoded(input) {
        request.insert(
            String::from("inputImage"),
            Json::Text(get_base64_str(input).to_owned()),
        );
    } else {
        request.insert(String::from("inputText"), Json::Text(input.to_owned()));
    }
    request.extend(inference_params.clone());
    request
}

pub fn transform_response(
    response_list: &[Object],
    model: &str,
    batch_data: Option<&[Object]>,
) -> Result<TitanEmbeddingResponse, TitanEmbeddingsError> {
    let mut transformed =
        response::transform_response(response_list, model, response::plain_embedding)?;
    let image_count = count_images(batch_data);
    transformed.usage.prompt_tokens_details =
        (image_count > 0).then_some(TitanPromptTokensDetails { image_count });
    Ok(transformed)
}

fn count_images(batch_data: Option<&[Object]>) -> u32 {
    let Some(batch) = batch_data else {
        return 0;
    };
    let mut count = 0_u32;
    for request_data in batch {
        if request_data.contains_key("inputImage") {
            count = count.saturating_add(1);
        }
    }
    count
}
