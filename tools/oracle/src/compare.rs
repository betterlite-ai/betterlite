use crate::case::Compare;
use crate::http::Response;
use crate::oracle_comparison_contracts::Contracts;
use crate::router_probes::comparison;
use betterlite_oracle_runner::case::Ignore;
use betterlite_oracle_runner::compare::Difference;
use betterlite_oracle_runner::compare::value::{first_difference, remove_pointer};
use betterlite_oracle_runner::mock::ProviderRequest;

pub(crate) mod multipart;
mod upstream;

pub(crate) fn compare(
    left: &Response,
    right: &Response,
    left_requests: &[ProviderRequest],
    right_requests: &[ProviderRequest],
    compare: &Compare,
) -> Result<(), Difference> {
    compare_responses(left, right, compare)?;
    upstream::compare_provider_requests(left_requests, right_requests, compare)
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
    compare_response_body(left, right, compare, &difference)?;
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
    compare: &Compare,
    difference: &dyn Fn(&str, &str) -> Difference,
) -> Result<(), Difference> {
    let no_contracts = Contracts::default();
    let contracts = compare.contracts.as_deref().unwrap_or(&no_contracts);
    // a selector names a provider request, so it must never remove a response field
    let ignores: Vec<Ignore> = compare
        .ignore
        .iter()
        .filter(|ignored| !ignored.pointer.starts_with("/upstream/"))
        .cloned()
        .collect();
    let Some(body_difference) = comparison::compare(&comparison::Documents {
        reference: &left.body,
        rust: &right.body,
        numeric: &contracts.numeric,
        expected_difference: &contracts.expected_difference,
        ignore: &ignores,
        self_check: contracts.self_check,
        message: "response bodies differ",
    }) else {
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
    if binary_response {
        return Err(difference("/response/bytes", "response bodies differ"));
    }
    Err(body_difference)
}

pub(crate) fn compare_sdk(
    left: &serde_json::Value,
    right: &serde_json::Value,
    ignores: &[Ignore],
) -> Result<(), Difference> {
    let (mut left_value, mut right_value) = (left.clone(), right.clone());
    for ignored in ignores {
        remove_pointer(&mut left_value, &ignored.pointer);
        remove_pointer(&mut right_value, &ignored.pointer);
    }
    first_difference(&left_value, &right_value, "").map_or(Ok(()), |pointer| {
        Err(Difference {
            pointer,
            message: "SDK results or provider requests differ".to_owned(),
        })
    })
}
