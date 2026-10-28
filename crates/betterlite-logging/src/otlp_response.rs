use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use prost::Message as _;

use crate::python_json::dumps_object;

mod tests;

const JSON: &str = "application/json";
const PROTOBUF: &str = "application/x-protobuf";

// OTLP answers a rejected export with google.rpc.Status; the code stays at its zero default
#[derive(Clone, PartialEq, prost::Message)]
struct Status {
    #[prost(int32, tag = "1")]
    code: i32,
    #[prost(string, tag = "2")]
    message: String,
}

#[must_use]
pub fn encode_otlp_response(
    content_type: Option<&str>,
    error: Option<&str>,
) -> (Vec<u8>, &'static str) {
    let requested = content_type
        .filter(|requested| !requested.is_empty())
        .unwrap_or(PROTOBUF);
    let media_type = requested
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_lowercase();
    match (media_type == JSON, error) {
        (true, Some(message)) if !message.is_empty() => {
            let body = IndexMap::from([("message".to_owned(), Json::Text(message.to_owned()))]);
            (dumps_object(&body).into_bytes(), JSON)
        }
        (true, None | Some(_)) => (b"{}".to_vec(), JSON),
        (false, None) => (Vec::new(), PROTOBUF),
        (false, Some(message)) => {
            let status = Status {
                code: 0,
                message: message.to_owned(),
            };
            (status.encode_to_vec(), PROTOBUF)
        }
    }
}
