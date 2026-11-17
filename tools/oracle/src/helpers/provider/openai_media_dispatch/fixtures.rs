use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_oracle_helper_api::error::HelperError;
use serde_json::Value;

use super::failure;

pub(super) use betterlite_provider::openai_media_dispatch::test_support::{Fixture, Recorder};

pub(super) fn only_request(fixture: &Fixture) -> Result<OutboundRequest, Box<HelperError>> {
    let count = fixture.request_count();
    if count != 1 {
        return Err(failure(
            "Fixture",
            format!("one dispatch sends one request, found {count}"),
        ));
    }
    fixture
        .take_request()
        .ok_or_else(|| failure("Fixture", "the fixture recorded no request"))
}

pub(super) fn header(request: &OutboundRequest, name: &str) -> Option<String> {
    request
        .headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

pub(super) fn json_body(request: &OutboundRequest) -> Result<Value, Box<HelperError>> {
    let RequestBody::Json(bytes) = &request.body else {
        return Err(failure("Fixture", "the request body is not JSON"));
    };
    serde_json::from_slice(bytes).map_err(|error| failure("Fixture", error))
}

pub(super) fn multipart_view(request: &OutboundRequest) -> Result<Value, Box<HelperError>> {
    let RequestBody::Multipart { parts, .. } = &request.body else {
        return Err(failure("Fixture", "the request is not multipart"));
    };
    let mut rendered = Vec::with_capacity(parts.len());
    for part in parts {
        rendered.push(serde_json::json!({
            "name": part.name,
            "filename": part.file_name,
            "content_type": part.content_type,
            "body": String::from_utf8(part.data.clone())
                .map_err(|error| failure("Utf8", error))?,
        }));
    }
    Ok(Value::Array(rendered))
}
