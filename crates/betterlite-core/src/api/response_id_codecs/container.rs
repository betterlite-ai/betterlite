use std::str::from_utf8;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_openai::api::responses_shapes::DecodedResponseId;

use super::CodecDecoding;
use super::lenient;

type ContainerFields = (Option<String>, Option<String>, String);

#[must_use]
pub fn build_container_id(
    custom_llm_provider: Option<&str>,
    model_id: Option<&str>,
    container_id: &str,
) -> String {
    let provider = custom_llm_provider.unwrap_or("");
    let model = model_id.unwrap_or("");
    let assembled = format!(
        "litellm:custom_llm_provider:{provider};model_id:{model};container_id:{container_id}"
    );
    format!("cntr_{}", STANDARD.encode(assembled))
}

#[must_use]
pub fn decode_container_id(container_id: &str) -> DecodedResponseId {
    match decode_envelope(container_id) {
        Ok(decoded) => decoded,
        Err(source) => {
            tracing::debug!(error = %source, container_id, "Error decoding container_id");
            DecodedResponseId {
                custom_llm_provider: Some(None),
                model_id: Some(None),
                response_id: Some(container_id.to_owned()),
            }
        }
    }
}

fn decode_envelope(container_id: &str) -> Result<DecodedResponseId, lenient::TextDecodeError> {
    let fields = parse_container(container_id)?;
    Ok(match fields {
        Some((custom_llm_provider, model_id, original)) => DecodedResponseId {
            custom_llm_provider: Some(custom_llm_provider),
            model_id: Some(model_id),
            response_id: Some(original),
        },
        None => DecodedResponseId {
            custom_llm_provider: Some(None),
            model_id: Some(None),
            response_id: Some(container_id.to_owned()),
        },
    })
}

fn parse_container(
    container_id: &str,
) -> Result<Option<ContainerFields>, lenient::TextDecodeError> {
    if !container_id.starts_with("cntr_") {
        return Ok(None);
    }
    let cleaned_id = container_id.replace("cntr_", "");
    let decoded_id = lenient::decode_text(&cleaned_id)?;
    let Some(rest) = decoded_id.strip_prefix("litellm:custom_llm_provider:") else {
        return Ok(None);
    };
    let Some((provider_part, remainder)) = rest.split_once(";model_id:") else {
        return Ok(None);
    };
    let Some((model_part, container_part)) = remainder.split_once(";container_id:") else {
        return Ok(None);
    };
    // Python's regex ends at the final newline, so one trailing newline is outside the captured id
    let original = container_part.strip_suffix('\n').unwrap_or(container_part);
    if provider_part.contains(';')
        || model_part.contains(';')
        || original.is_empty()
        || original.contains('\n')
    {
        return Ok(None);
    }
    let custom_llm_provider = match provider_part {
        "" | "None" => None,
        value => Some(value.to_owned()),
    };
    let model_id = match model_part {
        "" | "None" => None,
        value => Some(value.to_owned()),
    };
    Ok(Some((custom_llm_provider, model_id, original.to_owned())))
}

pub(super) fn decode_entry(bytes: &[u8]) -> CodecDecoding {
    let Ok(container_id) = from_utf8(bytes) else {
        tracing::debug!("Error decoding container_id: the id is not UTF-8");
        return CodecDecoding {
            custom_llm_provider: None,
            model_id: None,
            restored_id: None,
        };
    };
    let decoded = decode_container_id(container_id);
    CodecDecoding {
        custom_llm_provider: decoded.custom_llm_provider.flatten(),
        model_id: decoded.model_id.flatten(),
        restored_id: decoded.response_id,
    }
}
