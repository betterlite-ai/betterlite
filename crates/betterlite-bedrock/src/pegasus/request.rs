use crate::image_models::params::{Field, Params};
use crate::pegasus::error::PegasusError;
use crate::pegasus::media::build_media_source;
use crate::pegasus::prompt::{PromptMessage, convert_messages_to_prompt};
use crate::pegasus::response_format::normalize_response_format;

pub fn transform_request(
    messages: &[PromptMessage],
    optional: &Params,
) -> Result<Params, PegasusError> {
    let mut request = Params::new();
    drop(request.insert(
        "inputPrompt".to_owned(),
        Field::Text(convert_messages_to_prompt(messages)?),
    ));
    if let Some(media_source) = build_media_source(optional)? {
        drop(request.insert("mediaSource".to_owned(), media_source));
    }
    for key in ["temperature", "maxOutputTokens"] {
        if let Some(value) = optional.get(key) {
            drop(request.insert(key.to_owned(), value.clone()));
        }
    }
    if let Some(value) = optional.get("responseFormat") {
        let transformed = normalize_response_format(value)?;
        if transformed.truthy() {
            drop(request.insert("responseFormat".to_owned(), transformed));
        }
    }
    Ok(request)
}
