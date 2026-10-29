use crate::case::{Compare, Ignore};
use crate::http::Response;
use crate::mock::ProviderRequest;

pub(crate) mod helper;
pub(crate) mod multipart;
mod value;

use multipart::{captured_parts, first_part_difference, masked_boundary};
use value::{first_difference, remove_pointer};

use helper::HelperOutput;

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
    }
    if let Some(difference) = compare_effect_secrets(left, right, ignores) {
        return Some(difference);
    }
    for ignored in ignores {
        remove_pointer(&mut left_value, &ignored.pointer);
        remove_pointer(&mut right_value, &ignored.pointer);
    }
    first_difference(&left_value, &right_value, "").map(|pointer| Difference {
        pointer,
        message: "helper results differ".to_owned(),
    })
}

fn compare_effect_secrets(
    left: &HelperOutput,
    right: &HelperOutput,
    ignores: &[Ignore],
) -> Option<Difference> {
    if left.effects.len() != right.effects.len() {
        return None;
    }
    left.effects
        .iter()
        .zip(&right.effects)
        .enumerate()
        .find_map(|(index, (left_effect, right_effect))| {
            if effect_secret_is_ignored(ignores, index)
                || left_effect.fields.same_secret(&right_effect.fields)
            {
                None
            } else {
                Some(Difference {
                    message: "helper results differ".to_owned(),
                    pointer: format!("/effects/{index}/fields/credential"),
                })
            }
        })
}

fn effect_secret_is_ignored(ignores: &[Ignore], index: usize) -> bool {
    let prefix = format!("/effects/{index}/fields/credential");
    ignores.iter().any(|ignore| {
        prefix == ignore.pointer || prefix.starts_with(&format!("{}/", ignore.pointer))
    })
}

pub(crate) fn compare(
    left: &Response,
    right: &Response,
    left_requests: &[ProviderRequest],
    right_requests: &[ProviderRequest],
    compare: &Compare,
) -> Result<(), Difference> {
    compare_responses(left, right, compare)?;
    compare_provider_requests(left_requests, right_requests, compare)
}

fn compare_responses(
    left: &Response,
    right: &Response,
    compare: &Compare,
) -> Result<(), Difference> {
    let difference = |pointer: &str, message: &str| Difference {
        pointer: pointer.to_owned(),
        message: message.to_owned(),
    };
    if left.status != right.status {
        return Err(difference("/status", "response statuses differ"));
    }
    compare_response_body(left, right, &compare.ignore, &difference)?;
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
    Ok(())
}

fn compare_response_body(
    left: &Response,
    right: &Response,
    ignores: &[Ignore],
    difference: &dyn Fn(&str, &str) -> Difference,
) -> Result<(), Difference> {
    let mut left_body = left.body.clone();
    let mut right_body = right.body.clone();
    for ignored in ignores {
        remove_pointer(&mut left_body, &ignored.pointer);
        remove_pointer(&mut right_body, &ignored.pointer);
    }
    let Some(body_pointer) = first_difference(&left_body, &right_body, "") else {
        return Ok(());
    };
    let binary_response = left
        .headers
        .get("content-type")
        .is_some_and(|value| value.starts_with("application/octet-stream"))
        && right
            .headers
            .get("content-type")
            .is_some_and(|value| value.starts_with("application/octet-stream"));
    let response_pointer = if binary_response {
        "/response/bytes"
    } else {
        &body_pointer
    };
    Err(difference(response_pointer, "response bodies differ"))
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
        compare_provider_request(index, left_request, right_request, compare, &difference)?;
    }
    Ok(())
}

fn compare_provider_request(
    index: usize,
    left: &ProviderRequest,
    right: &ProviderRequest,
    compare: &Compare,
    difference: &dyn Fn(&str, &str) -> Difference,
) -> Result<(), Difference> {
    if left.method != right.method || left.path != right.path {
        return Err(difference(
            &format!("/upstream/{index}"),
            "provider request targets differ",
        ));
    }
    // two decoded captures compare part by part, so only their random boundaries may differ
    let body_pointer = match (captured_parts(left), captured_parts(right)) {
        (Some(left_parts), Some(right_parts)) => first_part_difference(&left_parts, &right_parts)
            .map(|part| format!("/upstream/{index}{part}")),
        (Some(_) | None, _) => (left.body != right.body).then(|| format!("/upstream/{index}/body")),
    };
    if let Some(pointer) = body_pointer {
        return Err(difference(&pointer, "provider request bodies differ"));
    }
    for header in &compare.upstream_headers {
        let name = header.to_ascii_lowercase();
        let left_value = left.headers.get(&name).map(|value| masked_boundary(value));
        if left_value != right.headers.get(&name).map(|value| masked_boundary(value)) {
            return Err(difference(
                &format!("/upstream/{index}/headers/{name}"),
                "provider header values differ",
            ));
        }
    }
    Ok(())
}
