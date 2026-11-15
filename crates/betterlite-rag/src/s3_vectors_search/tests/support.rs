use betterlite_models::decisions::Json;
use betterlite_provider::api::vector_store_contracts::config::{Headers, Params};
use indexmap::IndexMap;

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

pub(super) fn params(text: &str) -> Params {
    serde_json::from_str(text).unwrap()
}

pub(super) fn object(text: &str) -> IndexMap<String, Json> {
    let Json::Object(fields) = json(text) else {
        panic!("the fixture is not an object");
    };
    fields
}

pub(super) fn reply_headers() -> Headers {
    Headers::from([("x-request-id".to_owned(), "abc".to_owned())])
}
