use serde::{Deserialize, Serialize};

pub const MANAGEMENT_V1_PREFIX: &str = "/management/v1";

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ProblemDetail {
    pub r#type: String,
    pub title: String,
    pub status: u16,
    pub detail: String,
    #[serde(default)]
    pub allowed: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct PageLinks {
    #[serde(rename = "self", alias = "self_link")]
    pub self_link: String,
    #[serde(default)]
    pub prev: Option<String>,
    #[serde(default)]
    pub next: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct PageMeta {
    pub page: u64,
    pub page_size: u64,
    pub has_more: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct FacetListResponse {
    pub data: Vec<String>,
    pub meta: PageMeta,
    pub links: PageLinks,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ListMeta {
    pub total_count: u64,
    pub page: u64,
    pub page_size: u64,
    pub total_pages: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ListLinks {
    #[serde(rename = "self", alias = "self_link")]
    pub self_link: String,
    pub first: String,
    #[serde(default)]
    pub prev: Option<String>,
    #[serde(default)]
    pub next: Option<String>,
    pub last: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ResourceResponse<T> {
    pub data: T,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ListResponse<T> {
    pub data: Vec<T>,
    pub meta: ListMeta,
    pub links: ListLinks,
}

#[cfg(test)]
mod tests {
    use super::{
        FacetListResponse, ListResponse, PageLinks, PageMeta, ProblemDetail, ResourceResponse,
    };

    #[test]
    fn aliases_and_nullable_links_keep_wire_shape() {
        let links: PageLinks = serde_json::from_value(serde_json::json!({
            "self_link": "/management/v1/items",
            "prev": null,
            "next": null
        }))
        .expect("links decode");
        assert_eq!(
            serde_json::to_value(links).expect("links encode"),
            serde_json::json!({
                "self": "/management/v1/items", "prev": null, "next": null
            })
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
            data: serde_json::json!({"id": "nested"}),
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
    }

    #[test]
    fn problem_allowed_is_nullable_and_can_be_empty() {
        let absent: ProblemDetail = serde_json::from_value(serde_json::json!({
            "type": "urn:litellm:error:test", "title": "Test", "status": 400, "detail": "bad"
        }))
        .expect("problem decode");
        let empty: ProblemDetail = serde_json::from_value(serde_json::json!({
            "type": "urn:litellm:error:test", "title": "Test", "status": 400, "detail": "bad", "allowed": []
        }))
        .expect("problem decode");
        assert_eq!(absent.allowed, None);
        assert_eq!(empty.allowed, Some(Vec::new()));
    }
}
