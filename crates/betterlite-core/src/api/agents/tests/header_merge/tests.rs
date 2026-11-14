#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::api::agents::merge_agent_headers;

fn headers(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn both_sides_absent_merge_to_none() {
    assert_eq!(merge_agent_headers(None, None), None);
}

#[test]
fn both_sides_empty_merge_to_none() {
    let empty = BTreeMap::new();
    assert_eq!(merge_agent_headers(Some(&empty), Some(&empty)), None);
}

#[test]
fn an_empty_dynamic_map_without_static_headers_merges_to_none() {
    let empty = BTreeMap::new();
    assert_eq!(merge_agent_headers(Some(&empty), None), None);
}

#[test]
fn dynamic_headers_survive_without_static_headers() {
    let dynamic = headers(&[("authorization", "Bearer client"), ("x-trace", "t1")]);
    assert_eq!(
        merge_agent_headers(Some(&dynamic), None),
        Some(dynamic.clone())
    );
}

#[test]
fn static_headers_are_returned_without_dynamic_headers() {
    let static_headers = headers(&[("Authorization", "Bearer admin")]);
    assert_eq!(
        merge_agent_headers(None, Some(&static_headers)),
        Some(static_headers.clone())
    );
}

#[test]
fn an_empty_static_map_leaves_the_dynamic_headers_alone() {
    let dynamic = headers(&[("authorization", "Bearer client")]);
    let static_headers = BTreeMap::new();
    assert_eq!(
        merge_agent_headers(Some(&dynamic), Some(&static_headers)),
        Some(dynamic.clone())
    );
}

#[test]
fn an_empty_dynamic_map_keeps_the_static_headers() {
    let dynamic = BTreeMap::new();
    let static_headers = headers(&[("Authorization", "Bearer admin")]);
    assert_eq!(
        merge_agent_headers(Some(&dynamic), Some(&static_headers)),
        Some(static_headers.clone())
    );
}

#[rstest]
#[case::same_case("Authorization", "Authorization")]
#[case::dynamic_lower("authorization", "Authorization")]
#[case::static_lower("Authorization", "authorization")]
fn static_headers_win_a_collision_whatever_their_casing(
    #[case] dynamic_name: &str,
    #[case] static_name: &str,
) {
    let dynamic = headers(&[(dynamic_name, "Bearer client")]);
    let static_headers = headers(&[(static_name, "Bearer admin")]);
    assert_eq!(
        merge_agent_headers(Some(&dynamic), Some(&static_headers)),
        Some(headers(&[(static_name, "Bearer admin")]))
    );
}

#[test]
fn a_static_header_replaces_only_its_dynamic_counterpart() {
    let dynamic = headers(&[("authorization", "Bearer client"), ("x-trace", "t1")]);
    let static_headers = headers(&[("AUTHORIZATION", "Bearer admin")]);
    assert_eq!(
        merge_agent_headers(Some(&dynamic), Some(&static_headers)),
        Some(headers(&[
            ("AUTHORIZATION", "Bearer admin"),
            ("x-trace", "t1")
        ]))
    );
}

#[test]
fn every_static_header_strips_its_dynamic_counterpart() {
    let dynamic = headers(&[("x-a", "dynamic-a"), ("x-b", "dynamic-b")]);
    let static_headers = headers(&[("X-A", "static-a"), ("X-B", "static-b")]);
    assert_eq!(
        merge_agent_headers(Some(&dynamic), Some(&static_headers)),
        Some(static_headers.clone())
    );
}
