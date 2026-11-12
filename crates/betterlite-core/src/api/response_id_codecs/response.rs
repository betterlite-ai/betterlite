use std::str::from_utf8;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_openai::api::responses_shapes::DecodedResponseId;

use super::CodecDecoding;
use super::lenient;

#[must_use]
pub fn build_response_id(
    custom_llm_provider: Option<&str>,
    model_id: Option<&str>,
    response_id: &str,
) -> String {
    let provider = custom_llm_provider.unwrap_or("None");
    let model = model_id.unwrap_or("None");
    let assembled = format!(
        "litellm:custom_llm_provider:{provider};model_id:{model};response_id:{response_id}"
    );
    format!("resp_{}", STANDARD.encode(assembled))
}

#[must_use]
pub fn decode_response_id(response_id: &str) -> DecodedResponseId {
    match decode_envelope(response_id) {
        Ok(decoded) => decoded,
        Err(source) => {
            tracing::debug!(error = %source, response_id, "Error decoding response_id");
            DecodedResponseId {
                custom_llm_provider: Some(None),
                model_id: Some(None),
                response_id: Some(response_id.to_owned()),
            }
        }
    }
}

fn decode_envelope(response_id: &str) -> Result<DecodedResponseId, lenient::TextDecodeError> {
    let cleaned_id = response_id.replace("resp_", "");
    let decoded_id = lenient::decode_text(&cleaned_id)?;
    let mut parts = decoded_id.split(';');
    let (Some(provider_part), Some(model_part), Some(response_part)) =
        (parts.next(), parts.next(), parts.next())
    else {
        return Ok(DecodedResponseId {
            custom_llm_provider: Some(None),
            model_id: Some(None),
            response_id: Some(response_id.to_owned()),
        });
    };
    Ok(DecodedResponseId {
        custom_llm_provider: Some(Some(
            provider_part.replace("litellm:custom_llm_provider:", ""),
        )),
        model_id: Some(Some(model_part.replace("model_id:", ""))),
        response_id: Some(response_part.replace("response_id:", "")),
    })
}

#[must_use]
pub fn is_encoded_response_id(response_id: &str) -> bool {
    let decoded = decode_response_id(response_id);
    let provider = decoded.custom_llm_provider.flatten();
    let model = decoded.model_id.flatten();
    provider.is_some() || model.is_some()
}

#[must_use]
pub fn get_model_id_from_response_id(response_id: Option<&str>) -> Option<String> {
    let decoded = response_id.map(decode_response_id);
    decoded
        .and_then(|value| value.model_id)
        .flatten()
        .filter(|model_id| !model_id.is_empty())
}

#[must_use]
pub fn decode_previous_response_id(previous_response_id: &str) -> String {
    match decode_response_id(previous_response_id).response_id {
        Some(response_id) => response_id,
        None => previous_response_id.to_owned(),
    }
}

pub(super) fn decode_entry(bytes: &[u8]) -> CodecDecoding {
    let Ok(response_id) = from_utf8(bytes) else {
        tracing::debug!("Error decoding response_id: the id is not UTF-8");
        return CodecDecoding {
            custom_llm_provider: None,
            model_id: None,
            restored_id: None,
        };
    };
    let decoded = decode_response_id(response_id);
    CodecDecoding {
        custom_llm_provider: decoded.custom_llm_provider.flatten(),
        model_id: decoded.model_id.flatten(),
        restored_id: decoded.response_id,
    }
}
