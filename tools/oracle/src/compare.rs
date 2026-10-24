use std::collections::BTreeSet;

use serde_json::{Map, Value};
use thiserror::Error;

use crate::case::Compare;
use crate::http::Response;
use crate::mock::ProviderRequest;

#[derive(Debug, Error)]
#[error("first differing JSON pointer {pointer}: {message}")]
pub(crate) struct Difference {
    pub(crate) pointer: String,
    pub(crate) message: String,
}

pub(crate) fn compare(
    left: &Response,
    right: &Response,
    left_requests: &[ProviderRequest],
    right_requests: &[ProviderRequest],
    compare: &Compare,
) -> Result<(), Difference> {
    if left.status != right.status {
        return Err(difference("/status", "response statuses differ"));
    }
    let mut left_body = left.body.clone();
    let mut right_body = right.body.clone();
    for ignored in &compare.ignore {
        remove_pointer(&mut left_body, &ignored.pointer);
        remove_pointer(&mut right_body, &ignored.pointer);
    }
    if let Some(pointer) = first_difference(&left_body, &right_body, "") {
        return Err(difference(&pointer, "response bodies differ"));
    }
    for header in &compare.headers {
        let name = header.to_ascii_lowercase();
        if left.headers.get(&name) != right.headers.get(&name) {
            return Err(difference(
                &format!("/headers/{name}"),
                "response header values differ",
            ));
        }
    }
    for header in &compare.headers_present {
        let name = header.to_ascii_lowercase();
        if !left.headers.contains_key(&name) || !right.headers.contains_key(&name) {
            return Err(difference(
                &format!("/headers/{name}"),
                "response header is missing",
            ));
        }
    }
    compare_provider_requests(left_requests, right_requests, compare)
}

fn compare_provider_requests(
    left: &[ProviderRequest],
    right: &[ProviderRequest],
    compare: &Compare,
) -> Result<(), Difference> {
    if left.len() != right.len() {
        return Err(difference("/upstream", "provider request counts differ"));
    }
    for (index, (left_request, right_request)) in left.iter().zip(right).enumerate() {
        if left_request.method != right_request.method || left_request.path != right_request.path {
            return Err(difference(
                &format!("/upstream/{index}"),
                "provider request targets differ",
            ));
        }
        if left_request.body != right_request.body {
            return Err(difference(
                &format!("/upstream/{index}/body"),
                "provider request bodies differ",
            ));
        }
        for header in &compare.upstream_headers {
            let name = header.to_ascii_lowercase();
            if left_request.headers.get(&name) != right_request.headers.get(&name) {
                return Err(difference(
                    &format!("/upstream/{index}/headers/{name}"),
                    "provider header values differ",
                ));
            }
        }
    }
    Ok(())
}

fn remove_pointer(value: &mut Value, pointer: &str) {
    if pointer.is_empty() {
        *value = Value::Null;
        return;
    }
    let Some((head, tail)) = pointer
        .strip_prefix('/')
        .and_then(|rest| rest.split_once('/'))
    else {
        if let Some(object) = value.as_object_mut() {
            object.remove(&unescape(pointer.trim_start_matches('/')));
        }
        return;
    };
    let key = unescape(head);
    match value {
        Value::Object(object) => {
            if let Some(child) = object.get_mut(&key) {
                remove_pointer(child, &format!("/{tail}"));
            }
        }
        Value::Array(array) => {
            if let Ok(index) = key.parse::<usize>()
                && let Some(child) = array.get_mut(index)
            {
                remove_pointer(child, &format!("/{tail}"));
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn unescape(value: &str) -> String {
    value.replace("~1", "/").replace("~0", "~")
}

fn first_difference(left: &Value, right: &Value, pointer: &str) -> Option<String> {
    if left == right {
        return None;
    }
    match (left, right) {
        (Value::Object(left_object), Value::Object(right_object)) => {
            let keys = left_object
                .keys()
                .chain(right_object.keys())
                .collect::<BTreeSet<_>>();
            keys.into_iter().find_map(|key| {
                let child = format!("{pointer}/{}", key.replace('~', "~0").replace('/', "~1"));
                first_difference_optional(left_object.get(key), right_object.get(key), &child)
            })
        }
        (Value::Array(left_array), Value::Array(right_array)) => {
            let length = left_array.len().max(right_array.len());
            (0..length).find_map(|index| {
                let child = format!("{pointer}/{index}");
                first_difference_optional(left_array.get(index), right_array.get(index), &child)
            })
        }
        _ => Some(if pointer.is_empty() {
            "/".to_owned()
        } else {
            pointer.to_owned()
        }),
    }
}

fn first_difference_optional(
    left: Option<&Value>,
    right: Option<&Value>,
    pointer: &str,
) -> Option<String> {
    match (left, right) {
        (Some(left_value), Some(right_value)) => first_difference(left_value, right_value, pointer),
        (None, None) => None,
        _ => Some(pointer.to_owned()),
    }
}

fn difference(pointer: &str, message: &str) -> Difference {
    Difference {
        pointer: pointer.to_owned(),
        message: message.to_owned(),
    }
}

pub(crate) fn response_document(response: &Response) -> Value {
    let mut document = Map::new();
    document.insert("status".to_owned(), Value::from(response.status));
    document.insert(
        "headers".to_owned(),
        serde_json::to_value(&response.headers).unwrap_or(Value::Null),
    );
    document.insert("body".to_owned(), response.body.clone());
    Value::Object(document)
}
