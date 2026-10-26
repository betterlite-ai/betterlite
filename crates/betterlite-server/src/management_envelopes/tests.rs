#![cfg(test)]

use super::{
    FacetListResponse, ListResponse, PageLinks, PageMeta, ProblemDetail, ResourceResponse,
};

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
}

#[test]
fn facet_and_counted_envelopes_have_distinct_metadata() {
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
    let resource = ResourceResponse {
        data: NestedResource {
            id: "nested".to_owned(),
        },
    };
    let list = ListResponse {
        data: vec![resource.data],
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
    assert_eq!(facet.data, Vec::<String>::new());
    assert_eq!(list.data.len(), 1);
    assert_eq!(list.data[0].id, "nested");
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
}
