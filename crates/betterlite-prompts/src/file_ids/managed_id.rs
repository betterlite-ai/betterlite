use base64::Engine as _;
use base64::engine::general_purpose::STANDARD_NO_PAD;

const FILE_ID_PREFIX: &str = "litellm_proxy";
const MANAGED_PREFIX: &str = "litellm_proxy:";
const OUTPUT_FILE_ID_FIELD: &str = "llm_output_file_id,";
const MODEL_FIELD: &str = ";model,";
const DECODED_ID_PREFIX: &str = "litellm:";

#[must_use]
pub fn get_format_from_file_id(file_id: Option<&str>) -> Option<String> {
    let present = file_id.filter(|value| !value.is_empty())?;
    let unified = match decode_base64_text(present) {
        Some(decoded) if decoded.starts_with(FILE_ID_PREFIX) => decoded,
        _ => present.to_owned(),
    };
    let rest = unified.strip_prefix(MANAGED_PREFIX)?;
    let end = rest.find(";unified_id")?;
    let format = rest.get(..end)?;
    // A format that carries a newline never matches LiteLLM's lazy `.*?`
    (!format.contains('\n')).then(|| format.to_owned())
}

#[must_use]
pub(crate) fn llm_output_file_id(file_id: &str) -> Option<String> {
    let decoded = decode_base64_text(file_id)?;
    if !decoded.starts_with(FILE_ID_PREFIX) {
        return None;
    }
    let (_, after) = decoded.split_once(OUTPUT_FILE_ID_FIELD)?;
    let value = after.split_once(';').map_or(after, |(value, _)| value);
    Some(value.to_owned())
}

#[must_use]
pub(crate) fn is_unified_vector_id(vector_id: &str) -> bool {
    let Some(decoded) = decode_base64_text(vector_id) else {
        return false;
    };
    decoded.starts_with(MANAGED_PREFIX)
}

#[must_use]
pub(crate) fn provider_resource_id(unified_id: &str) -> Option<String> {
    let unified = match decode_base64_text(unified_id) {
        Some(decoded) if decoded.starts_with(MANAGED_PREFIX) => decoded,
        _ if unified_id.starts_with(MANAGED_PREFIX) => unified_id.to_owned(),
        _ => return None,
    };
    for marker in ["resource_id,", "vector_store_id,", "file_id,"] {
        if let Some(captured) = capture_until_semicolon(&unified, marker) {
            let provider = captured.trim();
            return (!provider.is_empty()).then(|| provider.to_owned());
        }
    }
    None
}

#[must_use]
pub(crate) fn original_file_id(file_id: &str) -> Option<String> {
    // The decoded model itself is unused; finding one marks the id as model-routed
    let model = decode_model_from_file_id(file_id)?;
    drop(model);
    Some(get_original_file_id(file_id))
}

fn decode_model_from_file_id(encoded_id: &str) -> Option<String> {
    let decoded = decode_prefixed_id(encoded_id)?;
    if !decoded.starts_with(DECODED_ID_PREFIX) || !decoded.contains(MODEL_FIELD) {
        return None;
    }
    let model = capture_until_semicolon(&decoded, MODEL_FIELD)?;
    Some(model.trim().to_owned())
}

fn get_original_file_id(encoded_id: &str) -> String {
    let Some(decoded) = decode_prefixed_id(encoded_id) else {
        return encoded_id.to_owned();
    };
    if !decoded.starts_with(DECODED_ID_PREFIX) || !decoded.contains(MODEL_FIELD) {
        return encoded_id.to_owned();
    }
    capture_before_field(&decoded, DECODED_ID_PREFIX, "model,")
        .map_or_else(|| encoded_id.to_owned(), str::to_owned)
}

fn decode_prefixed_id(encoded_id: &str) -> Option<String> {
    let body = encoded_id
        .strip_prefix("file-")
        .or_else(|| encoded_id.strip_prefix("batch_"))
        .unwrap_or(encoded_id);
    decode_base64_text(body)
}

fn decode_base64_text(encoded: &str) -> Option<String> {
    let normalized: String = encoded
        .chars()
        .filter_map(|character| match character {
            '-' => Some('+'),
            '_' => Some('/'),
            '+' | '/' => Some(character),
            alphanumeric if alphanumeric.is_ascii_alphanumeric() => Some(alphanumeric),
            _ => None,
        })
        .collect();
    let bytes = STANDARD_NO_PAD.decode(normalized).ok()?;
    String::from_utf8(bytes).ok()
}

fn capture_until_semicolon<'text>(text: &'text str, marker: &str) -> Option<&'text str> {
    text.match_indices(marker).find_map(|(index, _)| {
        let rest = text.get(index.saturating_add(marker.len())..)?;
        let captured = rest.split_once(';').map_or(rest, |(captured, _)| captured);
        (!captured.is_empty()).then_some(captured)
    })
}

fn capture_before_field<'text>(text: &'text str, start: &str, field: &str) -> Option<&'text str> {
    text.match_indices(start).find_map(|(index, _)| {
        let rest = text.get(index.saturating_add(start.len())..)?;
        let (captured, after) = rest.split_once(';')?;
        (!captured.is_empty() && after.starts_with(field)).then_some(captured)
    })
}
