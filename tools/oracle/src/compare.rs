use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::case::{Compare, Ignore};
use crate::http::Response;
use crate::mock::ProviderRequest;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub(crate) struct HelperOutput {
    pub(crate) ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) value: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<HelperError>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) warnings: Vec<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) effects: Vec<HelperEffect>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub(crate) struct HelperEffect {
    pub(crate) kind: String,
    pub(crate) fields: BTreeMap<String, Value>,
    #[serde(default)]
    pub(crate) secret_fields: BTreeSet<String>,
}

type OptionalValue = Option<Option<Value>>;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub(crate) struct HelperError {
    pub(crate) class: String,
    pub(crate) status: Option<u16>,
    #[serde(
        rename = "type",
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub(crate) error_type: OptionalValue,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub(crate) param: OptionalValue,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub(crate) code: OptionalValue,
    pub(crate) message: String,
}

#[derive(Debug, serde::Serialize)]
pub(crate) struct Difference {
    pub(crate) message: String,
    pub(crate) pointer: String,
}

pub(crate) fn compare_helper(
    left: &HelperOutput,
    right: &HelperOutput,
    ignores: &[Ignore],
) -> Option<Difference> {
    let mut left_value = serde_json::to_value(left).ok()?;
    let mut right_value = serde_json::to_value(right).ok()?;
    for ignored in ignores {
        let known = matches!(
            ignored.pointer.as_str(),
            "/ok"
                | "/value"
                | "/error"
                | "/error/class"
                | "/error/status"
                | "/error/type"
                | "/error/param"
                | "/error/code"
                | "/error/message"
                | "/warnings"
                | "/effects"
        );
        if left_value.pointer(&ignored.pointer).is_none()
            && right_value.pointer(&ignored.pointer).is_none()
            && !known
        {
            return Some(Difference {
                message: "compare.ignore points to no field".to_owned(),
                pointer: ignored.pointer.clone(),
            });
        }
        remove_pointer(&mut left_value, &ignored.pointer);
        remove_pointer(&mut right_value, &ignored.pointer);
    }
    first_difference(&left_value, &right_value, "").map(|pointer| Difference {
        pointer,
        message: "helper results differ".to_owned(),
    })
}

pub(crate) fn compare(
    left: &Response,
    right: &Response,
    left_requests: &[ProviderRequest],
    right_requests: &[ProviderRequest],
    compare: &Compare,
) -> Result<(), Difference> {
    let difference = |pointer: &str, message: &str| Difference {
        pointer: pointer.to_owned(),
        message: message.to_owned(),
    };
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
    let difference = |pointer: &str, message: &str| Difference {
        pointer: pointer.to_owned(),
        message: message.to_owned(),
    };
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
    if let Some((parent, key)) = pointer.rsplit_once('/') {
        let decoded = key.replace("~1", "/").replace("~0", "~");
        if let Some(Value::Object(object)) = value.pointer_mut(parent) {
            object.remove(&decoded);
            return;
        }
        if let Some(child) = value.pointer_mut(pointer) {
            *child = Value::Null;
        }
    }
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
        _ => Some(pointer.to_owned()),
    }
}
