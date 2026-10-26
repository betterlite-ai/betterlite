#![cfg(test)]

use super::{
    FacetListResponse, ListResponse, PageLinks, PageMeta, ProblemDetail, ResourceResponse,
};

#[derive(serde::Serialize)]
struct NestedResource {
    id: String,
}

#[test]
fn aliases_and_nullable_links_keep_wire_shape() {
    let links: PageLinks =
        serde_json::from_str(r#"{"self_link":"/management/v1/items","prev":null,"next":null}"#)
            .expect("links decode");
    assert_eq!(
        serde_json::to_string(&links).expect("links encode"),
        r#"{"self":"/management/v1/items","prev":null,"next":null}"#
    );

    let missing_neighbors: PageLinks =
        serde_json::from_str(r#"{"self":"/management/v1/items"}"#).expect("links decode");
    assert_eq!(
        serde_json::to_string(&missing_neighbors).expect("links encode"),
        r#"{"self":"/management/v1/items","prev":null,"next":null}"#
    );
}

#[test]
fn resource_serialization_remains_flat() {
    assert_eq!(
        serde_json::to_string(&ResourceResponse {
            data: NestedResource {
                id: "nested".to_owned(),
            },
        })
        .expect("resource encode"),
        r#"{"data":{"id":"nested"}}"#
    );
}

#[test]
fn facet_serialization_keeps_page_metadata() {
    let facet = FacetListResponse {
        data: Vec::new(),
        meta: PageMeta {
            page: 1,
            page_size: 10,
            has_more: false,
        },
        links: PageLinks {
            self_link: "/facets".to_owned(),
            prev: None,
            next: None,
        },
    };
    assert_eq!(
        serde_json::to_string(&facet).expect("facet encode"),
        r#"{"data":[],"meta":{"page":1,"page_size":10,"has_more":false},"links":{"self":"/facets","prev":null,"next":null}}"#
    );
}

#[test]
fn counted_list_serialization_keeps_count_metadata() {
    let list = ListResponse {
        data: vec![NestedResource {
            id: "nested".to_owned(),
        }],
        meta: super::ListMeta {
            total_count: 1,
            page: 1,
            page_size: 10,
            total_pages: 1,
        },
        links: super::ListLinks {
            self_link: "/items".to_owned(),
            first: "/items".to_owned(),
            prev: None,
            next: None,
            last: "/items".to_owned(),
        },
    };
    assert_eq!(
        serde_json::to_string(&list).expect("list encode"),
        r#"{"data":[{"id":"nested"}],"meta":{"total_count":1,"page":1,"page_size":10,"total_pages":1},"links":{"self":"/items","first":"/items","prev":null,"next":null,"last":"/items"}}"#
    );
}

#[test]
fn problem_allowed_is_nullable_and_can_be_empty() {
    let absent: ProblemDetail = serde_json::from_str(
        r#"{"type":"urn:litellm:error:test","title":"Test","status":400,"detail":"bad"}"#,
    )
    .expect("problem decode");
    let empty: ProblemDetail = serde_json::from_str(
        r#"{"type":"urn:litellm:error:test","title":"Test","status":400,"detail":"bad","allowed":[]}"#,
    )
    .expect("problem decode");
    assert_eq!(absent.allowed, None);
    assert_eq!(empty.allowed, Some(Vec::new()));
    assert_eq!(
        serde_json::to_string(&absent.problem_document()).expect("document encode"),
        r#"{"type":"urn:litellm:error:test","title":"Test","status":400,"detail":"bad"}"#
    );
    assert_eq!(
        serde_json::to_string(&empty.problem_document()).expect("document encode"),
        r#"{"type":"urn:litellm:error:test","title":"Test","status":400,"detail":"bad","allowed":[]}"#
    );
    assert_eq!(
        serde_json::to_string(&absent).expect("ordinary encode"),
        r#"{"type":"urn:litellm:error:test","title":"Test","status":400,"detail":"bad","allowed":null}"#
    );
}
