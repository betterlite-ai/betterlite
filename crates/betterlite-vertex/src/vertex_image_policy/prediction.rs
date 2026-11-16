use betterlite_models::decisions::{Extra, Json};
use betterlite_openai::api::image_contracts::records::ImageData;

use super::error::PredictionError;
use crate::vertex_imagen_generation::response::ImagenGenerationResponse;

const PREDICTIONS: &str = "predictions";
const IMAGE_KEY: &str = "bytesBase64Encoded";

const fn kind(value: &Json) -> &'static str {
    match value {
        Json::Null => "null",
        Json::Bool(_) => "a boolean",
        Json::Number(_) => "a number",
        Json::Text(_) => "a string",
        Json::List(_) => "a list",
        Json::Object(_) => "an object",
    }
}

// the reply's predictions list, None when the key is absent
fn prediction_list(reply: &Extra) -> Result<Option<&[Json]>, PredictionError> {
    let Some(value) = reply.get(PREDICTIONS) else {
        return Ok(None);
    };
    let Json::List(items) = value else {
        return Err(PredictionError::Shape {
            path: PREDICTIONS.to_owned(),
            found: kind(value),
        });
    };
    Ok(Some(items))
}

fn prediction_object(item: &Json, index: usize) -> Result<&Extra, PredictionError> {
    let Json::Object(fields) = item else {
        return Err(PredictionError::Shape {
            path: format!("{PREDICTIONS}[{index}]"),
            found: kind(item),
        });
    };
    Ok(fields)
}

// is_image_generation_response looks at the first prediction only, and an empty list is an IndexError
pub fn is_image_generation_response(reply: &Extra) -> Result<bool, PredictionError> {
    let Some(items) = prediction_list(reply)? else {
        return Ok(false);
    };
    let first = items.first().ok_or(PredictionError::EmptyPredictions)?;
    Ok(prediction_object(first, 0)?.contains_key(IMAGE_KEY))
}

// ast-grep-ignore: no-long-names
pub fn process_image_generation_response(
    reply: &Extra,
    response: &mut ImagenGenerationResponse,
) -> Result<(), PredictionError> {
    let items = prediction_list(reply)?.ok_or(PredictionError::MissingPredictions)?;
    let mut objects = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        objects.push(prediction_object(item, index)?);
    }
    let mut images = Vec::with_capacity(objects.len());
    for (index, fields) in objects.into_iter().enumerate() {
        let b64_json = match fields.get(IMAGE_KEY) {
            None => return Err(PredictionError::MissingImageKey { index }),
            Some(Json::Null) => None,
            Some(Json::Text(text)) => Some(text.clone()),
            Some(_) => return Err(PredictionError::ImageType { index }),
        };
        images.push(ImageData {
            b64_json,
            revised_prompt: None,
            url: None,
        });
    }
    response.data = images;
    Ok(())
}
