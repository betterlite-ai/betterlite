use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::api::batch_records::WireDecodeLimits;
use betterlite_openai::files::records::BinaryContent;
use betterlite_openai::xai_files::error::XaiFilesError;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{Map, Value, json};

use crate::helpers::OrHelperError as _;

use super::ContentSpec;

pub(super) const fn limits() -> WireDecodeLimits {
    WireDecodeLimits {
        max_json_bytes: 1 << 20,
        max_event_bytes: 1 << 10,
        max_items: 1000,
    }
}

pub(super) fn failure(error: &XaiFilesError) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: String::from("XaiFilesError"),
        status: error.status_code(),
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

pub(super) fn upstream(status: u16, body: &[u8], headers: &[(String, String)]) -> UpstreamResponse {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        let Ok(header_name) = HeaderName::try_from(name.as_str()) else {
            continue;
        };
        let Ok(header_value) = HeaderValue::try_from(value.as_str()) else {
            continue;
        };
        map.insert(header_name, header_value);
    }
    UpstreamResponse {
        status,
        headers: map,
        body: body.to_vec(),
    }
}

pub(super) fn value<T>(
    result: Result<T, XaiFilesError>,
) -> Result<Value, Box<crate::helpers::HelperError>>
where
    T: serde::Serialize,
{
    match result {
        Ok(value) => serde_json::to_value(&value).or_config_error(),
        Err(error) if error.status_code().is_some() => Err(failure(&error)),
        Err(_) => Ok(json!({"raised": "reply"})),
    }
}

pub(super) fn empty_list(url: &str) -> Map<String, Value> {
    Map::from_iter([
        ("url".to_owned(), Value::String(url.to_owned())),
        ("next".to_owned(), Value::Null),
        ("raised".to_owned(), Value::Null),
        ("files".to_owned(), Value::Array(Vec::new())),
        ("raised_list".to_owned(), Value::Null),
    ])
}

pub(super) fn content_call(spec: &ContentSpec) -> Value {
    let content = BinaryContent::from_response(upstream(spec.status, &spec.body, &spec.headers));
    let watched: Map<String, Value> = content
        .headers
        .into_iter()
        .map(|(name, value)| (name, Value::String(value)))
        .collect();
    json!({
        "status": content.status,
        "headers": Value::Object(watched),
        "body": content.body,
    })
}
