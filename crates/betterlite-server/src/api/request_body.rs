use betterlite_config::sections::request_body::Settings;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::request_body::{
    BodyKind, DEFAULT_MAX_REPAIR_BYTES, DecodedJson, JsonObject, MediaTypeKind, ParsedBody,
    RequestBodyError, RequestBodyInput, RequestBodyResult, classify_media_type, decode_json_body,
    form_body, is_opaque_audio_request, is_otlp_trace_request, parse_binary_body,
};

mod tests;

// the named decoder is the default repair policy; fuzz, replay and the default boundary all drive it
pub fn decode_request_body(
    bytes: &[u8],
    limits: &ApiLimits,
) -> Result<DecodedJson, RequestBodyError> {
    decode_json_body(bytes, limits, Some(DEFAULT_MAX_REPAIR_BYTES))
}

// the auth-time pre-read: opaque audio is forwarded byte for byte, so its cache is seeded empty without a read
pub fn prepare_request_body(
    input: &RequestBodyInput,
    settings: &Settings,
    limits: &ApiLimits,
) -> Result<RequestBodyResult, RequestBodyError> {
    if is_opaque_audio_request(&input.route_path, input.content_type()) {
        return Ok(RequestBodyResult {
            branch: BodyKind::OpaqueAudio,
            body: JsonObject::new(),
            body_read: false,
            parsed_body: Some(ParsedBody::from_client_body(JsonObject::new())),
        });
    }
    read_request_body(input, settings, limits)
}

pub fn read_request_body(
    input: &RequestBodyInput,
    settings: &Settings,
    limits: &ApiLimits,
) -> Result<RequestBodyResult, RequestBodyError> {
    if is_otlp_trace_request(&input.method, &input.route_path) {
        return Ok(RequestBodyResult {
            branch: BodyKind::Otlp,
            body: JsonObject::new(),
            body_read: false,
            parsed_body: None,
        });
    }
    if let Some(cached) = &input.cached_body {
        return Ok(RequestBodyResult {
            branch: BodyKind::Cached,
            body: cached.visible(),
            body_read: false,
            parsed_body: Some(cached.clone()),
        });
    }
    match classify_media_type(input.content_type()) {
        MediaTypeKind::Form => decode_form_body(input),
        MediaTypeKind::Binary => Ok(parsed_result(
            BodyKind::Binary,
            parse_binary_body(&input.body),
        )),
        MediaTypeKind::Json | MediaTypeKind::Other => decode_json_branch(input, settings, limits),
    }
}

// a configured cap takes the decoder's path with another bound, so both go through one parse
fn decode_json_branch(
    input: &RequestBodyInput,
    settings: &Settings,
    limits: &ApiLimits,
) -> Result<RequestBodyResult, RequestBodyError> {
    let decoded = if settings.max_repair_bytes == Some(DEFAULT_MAX_REPAIR_BYTES) {
        decode_request_body(&input.body, limits)?
    } else {
        decode_json_body(&input.body, limits, settings.max_repair_bytes)?
    };
    match decoded {
        DecodedJson::Unreadable => Ok(RequestBodyResult {
            branch: BodyKind::Json,
            body: JsonObject::new(),
            body_read: true,
            parsed_body: None,
        }),
        DecodedJson::Parsed { body, repaired } => {
            let branch = if repaired {
                BodyKind::RepairedJson
            } else {
                BodyKind::Json
            };
            Ok(parsed_result(branch, body))
        }
    }
}

fn parsed_result(branch: BodyKind, body: JsonObject) -> RequestBodyResult {
    RequestBodyResult {
        branch,
        parsed_body: Some(ParsedBody::from_client_body(body.clone())),
        body,
        body_read: true,
    }
}

fn decode_form_body(input: &RequestBodyInput) -> Result<RequestBodyResult, RequestBodyError> {
    let Some(fields) = input.form_fields.as_ref() else {
        return Err(RequestBodyError::UndecodedForm);
    };
    Ok(parsed_result(BodyKind::Form, form_body(fields)?))
}
