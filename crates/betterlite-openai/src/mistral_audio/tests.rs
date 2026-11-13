#![cfg(test)]

mod corpus;
mod mistral_audio;

use betterlite_http::proxy::UpstreamResponse;
use http::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{Map, Value};

use crate::api::speech_provider_contract::TextToSpeechRequestData;
use crate::openai_base_config::Object;

pub(super) fn object(text: &str) -> Object {
    serde_json::from_str(text).expect("a JSON object")
}

pub(super) fn response(status: u16, pairs: &[(&str, &str)], body: &[u8]) -> UpstreamResponse {
    let mut headers = HeaderMap::new();
    for (name, value) in pairs {
        headers.append(
            HeaderName::from_bytes(name.as_bytes()).expect("a header name"),
            HeaderValue::from_str(value).expect("a header value"),
        );
    }
    UpstreamResponse {
        status,
        headers,
        body: body.to_vec(),
    }
}

pub(super) fn speech_request(pairs: &[(&str, &str)]) -> TextToSpeechRequestData {
    let mut body = Map::new();
    for (name, value) in pairs {
        body.insert((*name).to_owned(), Value::String((*value).to_owned()));
    }
    TextToSpeechRequestData {
        dict_body: Some(body),
        ssml_body: None,
        headers: None,
    }
}
