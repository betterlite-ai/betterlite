use betterlite_config::sections::request_body::Settings;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::request_body::decode::{DecodedJson, decode_json_body};
use betterlite_core::api::request_body::{
    BodyKind, DEFAULT_MAX_REPAIR_BYTES, JsonObject, MediaTypeKind, ParsedBody, RequestBodyError,
    RequestBodyInput, RequestBodyResult, classify_media_type, form_body, is_opaque_audio_request,
    is_otlp_trace_request, parse_binary_body,
};

mod tests;

pub fn decode_request_body(
    bytes: &[u8],
    limits: &ApiLimits,
) -> Result<DecodedJson, RequestBodyError> {
    // the default repair policy; fuzz, replay and the default boundary all drive it
    decode_json_body(bytes, limits, Some(DEFAULT_MAX_REPAIR_BYTES))
}

pub fn read_request_body(
    input: &RequestBodyInput,
    settings: &Settings,
    limits: &ApiLimits,
) -> Result<RequestBodyResult, RequestBodyError> {
    // opaque audio is forwarded byte for byte, so its cache is seeded empty without a read
    if is_opaque_audio_request(&input.route_path, input.content_type()) {
        return Ok(unread_body(
            BodyKind::OpaqueAudio,
            Some(ParsedBody::from_client_body(JsonObject::new())),
        ));
    }
    if is_otlp_trace_request(&input.method, &input.route_path) {
        return Ok(unread_body(BodyKind::Otlp, None));
    }
    if let Some(cached) = &input.cached_body {
        return Ok(unread_body(BodyKind::Cached, Some(cached.clone())));
    }
    match classify_media_type(input.content_type()) {
        MediaTypeKind::Form => decode_form_body(input),
        MediaTypeKind::Binary => Ok(parsed_result(
            BodyKind::Binary,
            parse_binary_body(&input.body),
        )),
        MediaTypeKind::Json | MediaTypeKind::Other => {
            decode_json_branch(input, settings.max_repair_bytes, limits)
        }
    }
}

// a configured cap takes the decoder's path with another bound, so both go through one parse
fn decode_json_branch(
    input: &RequestBodyInput,
    max_repair_bytes: Option<u64>,
    limits: &ApiLimits,
) -> Result<RequestBodyResult, RequestBodyError> {
    let decoded = if max_repair_bytes == Some(DEFAULT_MAX_REPAIR_BYTES) {
        decode_request_body(&input.body, limits)?
    } else {
        decode_json_body(&input.body, limits, max_repair_bytes)?
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

fn unread_body(branch: BodyKind, parsed_body: Option<ParsedBody>) -> RequestBodyResult {
    RequestBodyResult {
        branch,
        body: JsonObject::new(),
        body_read: false,
        parsed_body,
    }
}

fn decode_form_body(input: &RequestBodyInput) -> Result<RequestBodyResult, RequestBodyError> {
    let Some(fields) = input.form_fields.as_ref() else {
        return Err(RequestBodyError::UndecodedForm);
    };
    Ok(parsed_result(BodyKind::Form, form_body(fields)?))
}
