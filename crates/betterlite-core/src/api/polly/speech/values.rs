use betterlite_openai::openai_base_config::Object;

use super::error::SpeechError;

// LiteLLM reads these slots with dict.get and treats a missing or null value the same way
pub(super) fn text_field<'params>(
    params: &'params Object,
    field: &'static str,
) -> Result<Option<&'params str>, SpeechError> {
    let Some(value) = params.get(field) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    value
        .as_str()
        .map(Some)
        .ok_or(SpeechError::WrongType { field })
}
