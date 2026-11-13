#![cfg(test)]

use betterlite_models::decisions::Json;

use crate::grounding_requests::{GroundingRequests, calculate_grounding_requests};

fn items(source: &str) -> Vec<Json> {
    serde_json::from_str(source).unwrap()
}

fn counted(source: &str) -> GroundingRequests {
    calculate_grounding_requests(&items(source))
}

#[test]
fn no_grounding_metadata_reports_no_requests() {
    assert_eq!(
        GroundingRequests::default(),
        calculate_grounding_requests(&[])
    );
    assert_eq!(GroundingRequests::default(), counted(r#"[{"n": 1}]"#));
}

#[test]
fn duplicate_queries_count_once_across_candidates() {
    let requests =
        counted(r#"[{"webSearchQueries": ["a", "b"]}, {"webSearchQueries": ["b", "a"]}]"#);
    assert_eq!(requests.web_search_requests, Some(2));
    assert_eq!(requests.google_maps_grounding_requests, None);
}

#[test]
fn duplicate_queries_within_one_candidate_count_once() {
    let requests = counted(r#"[{"webSearchQueries": ["a", "a", "a"]}]"#);
    assert_eq!(requests.web_search_requests, Some(1));
}

#[test]
fn empty_and_nonstring_queries_are_ignored() {
    let requests = counted(r#"[{"webSearchQueries": ["", 5, null, ["a"], "a"]}]"#);
    assert_eq!(requests.web_search_requests, Some(1));
}

#[test]
fn a_non_list_query_field_reports_nothing() {
    let requests = counted(r#"[{"webSearchQueries": "a"}]"#);
    assert_eq!(requests.web_search_requests, None);
}

#[test]
fn a_maps_chunk_floors_the_maps_requests_at_one() {
    let requests = counted(r#"[{"groundingChunks": [{"maps": {}}]}]"#);
    assert_eq!(requests.web_search_requests, None);
    assert_eq!(requests.google_maps_grounding_requests, Some(1));
}

#[test]
fn maps_only_queries_attribute_to_maps() {
    let requests = counted(
        r#"[{"webSearchQueries": ["m1", "m2"], "groundingChunks": [{"maps": {"uri": "u"}}]}]"#,
    );
    assert_eq!(requests.web_search_requests, None);
    assert_eq!(requests.google_maps_grounding_requests, Some(2));
}

#[test]
fn mixed_web_and_maps_chunks_attribute_queries_to_search() {
    let requests =
        counted(r#"[{"webSearchQueries": ["q"], "groundingChunks": [{"web": {}, "maps": {}}]}]"#);
    assert_eq!(requests.web_search_requests, Some(1));
    assert_eq!(requests.google_maps_grounding_requests, Some(1));
}

#[test]
fn a_widget_token_floors_the_maps_requests_at_one() {
    let requests = counted(r#"[{"googleMapsWidgetContextToken": "token"}]"#);
    assert_eq!(requests.web_search_requests, None);
    assert_eq!(requests.google_maps_grounding_requests, Some(1));
}

#[test]
fn an_empty_widget_token_does_not_floor_the_maps_requests() {
    let requests = counted(r#"[{"googleMapsWidgetContextToken": ""}]"#);
    assert_eq!(requests.google_maps_grounding_requests, None);
}

#[test]
fn maps_queries_deduplicate_across_candidates() {
    let requests = counted(
        r#"[{"webSearchQueries": ["m"], "groundingChunks": [{"maps": {}}]},
            {"webSearchQueries": ["m"], "googleMapsWidgetContextToken": "t"}]"#,
    );
    assert_eq!(requests.google_maps_grounding_requests, Some(1));
}

#[test]
fn plain_and_maps_items_keep_their_own_query_sets() {
    let requests = counted(
        r#"[{"webSearchQueries": ["m"], "groundingChunks": [{"maps": {}}]},
            {"webSearchQueries": ["w", "m"]}]"#,
    );
    assert_eq!(requests.web_search_requests, Some(2));
    assert_eq!(requests.google_maps_grounding_requests, Some(1));
}

#[test]
fn a_non_list_chunk_field_reports_no_kinds() {
    let requests = counted(r#"[{"groundingChunks": {"web": {}}}]"#);
    assert_eq!(requests.google_maps_grounding_requests, None);
}

#[test]
fn non_mapping_chunk_entries_are_ignored() {
    let requests = counted(r#"[{"groundingChunks": [null, 5, {"maps": {}}]}]"#);
    assert_eq!(requests.google_maps_grounding_requests, Some(1));
}

#[test]
fn non_mapping_items_are_ignored() {
    assert_eq!(
        GroundingRequests::default(),
        counted(r#"[null, 5, "webSearchQueries", [1]]"#)
    );
}
