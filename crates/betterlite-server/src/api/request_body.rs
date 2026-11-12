use betterlite_config::sections::request_body::Settings;
use betterlite_core::api::api_boundaries::{ApiBoundariesInput, ApiLimits, JsonBoundary};
use betterlite_core::api::request_body::{
    BodyKind, DecodedJson, JsonObject, MediaTypeKind, ParsedBody, ParsedJson, RequestBodyError,
    RequestBodyInput, RequestBodyResult, classify_media_type, form_body, is_opaque_audio_request,
    is_otlp_trace_request, parse_binary_body, parse_json_bytes, parse_json_text,
};
use std::str;

mod tests;

pub fn decode_request_body(
    bytes: &[u8],
    limits: &ApiLimits,
) -> Result<DecodedJson, RequestBodyError> {
    limits.check(ApiBoundariesInput::Json(JsonBoundary {
        known_bytes: bytes.len(),
    }))?;
    if bytes.is_empty() {
        return Ok(DecodedJson::Parsed {
            body: JsonObject::new(),
            repaired: false,
        });
    }
    match parse_json_bytes(bytes) {
        ParsedJson::Object(body) => Ok(DecodedJson::Parsed {
            body,
            repaired: false,
        }),
        ParsedJson::NotObject => Err(RequestBodyError::NotAnObject),
        ParsedJson::Invalid => repair_json_body(bytes),
    }
}

fn repair_json_body(bytes: &[u8]) -> Result<DecodedJson, RequestBodyError> {
    let Ok(text) = str::from_utf8(bytes) else {
        return Ok(DecodedJson::Unreadable);
    };
    match parse_json_text(text) {
        ParsedJson::Object(body) => Ok(DecodedJson::Parsed {
            body,
            repaired: true,
        }),
        ParsedJson::NotObject => Err(RequestBodyError::NotAnObject),
        ParsedJson::Invalid => Err(RequestBodyError::InvalidJson),
    }
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

fn decode_json_branch(
    input: &RequestBodyInput,
    max_repair_bytes: Option<u64>,
    limits: &ApiLimits,
) -> Result<RequestBodyResult, RequestBodyError> {
    let decoded = decode_request_body(&input.body, limits)?;
    let repair_ran = matches!(
        decoded,
        DecodedJson::Unreadable | DecodedJson::Parsed { repaired: true, .. }
    );
    // the pinned proxy skips the repair above the configured size and answers 400 instead
    let over_cap = max_repair_bytes
        .is_some_and(|cap| usize::try_from(cap).is_ok_and(|limit| input.body.len() > limit));
    if repair_ran && over_cap {
        return Err(RequestBodyError::InvalidJson);
    }
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

const fn unread_body(branch: BodyKind, parsed_body: Option<ParsedBody>) -> RequestBodyResult {
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
