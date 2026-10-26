use betterlite_server::management_envelopes::{FacetListResponse, ListLinks, ListMeta, ListResponse, PageLinks, PageMeta, ProblemDetail, ResourceResponse};
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Problem { r#type: String, title: String, status: u16, detail: String, #[serde(default)] allowed: Option<Vec<String>>, #[serde(default)] document: bool },
    Facet { data: Vec<String>, page: u64, page_size: u64, has_more: bool, #[serde(rename = "self", alias = "self_link")] self_link: String, #[serde(default)] prev: Option<String>, #[serde(default)] next: Option<String> },
    List { data: Vec<Value>, total_count: u64, page: u64, page_size: u64, total_pages: u64, #[serde(rename = "self", alias = "self_link")] self_link: String, first: String, #[serde(default)] prev: Option<String>, #[serde(default)] next: Option<String>, last: String },
    Resource { data: Value },
}

pub(crate) struct Op;

fn serialize<T: serde::Serialize>(value: T) -> crate::helpers::HelperResult {
    serde_json::to_value(value).map_err(|error| {
        Box::new(crate::helpers::HelperError {
            class: "SerializationError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let output = match input {
            Input::Problem { r#type, title, status, detail, allowed, document } => {
                let problem = ProblemDetail { r#type: r#type.clone(), title: title.clone(), status: *status, detail: detail.clone(), allowed: allowed.clone() };
                let mut value = serde_json::to_value(problem).map_err(|error| {
                    Box::new(crate::helpers::HelperError {
                        class: "SerializationError".to_owned(),
                        status: None,
                        error_type: None,
                        param: None,
                        code: None,
                        message: error.to_string(),
                    })
                })?;
                if *document && allowed.is_none() {
                    if let Value::Object(fields) = &mut value { fields.remove("allowed"); }
                }
                value
            }
            Input::Facet { data, page, page_size, has_more, self_link, prev, next } => serialize(FacetListResponse {
                data: data.clone(), meta: PageMeta { page: *page, page_size: *page_size, has_more: *has_more }, links: PageLinks { self_link: self_link.clone(), prev: prev.clone(), next: next.clone() },
            }),
            Input::List { data, total_count, page, page_size, total_pages, self_link, first, prev, next, last } => serialize(ListResponse {
                data: data.clone(), meta: ListMeta { total_count: *total_count, page: *page, page_size: *page_size, total_pages: *total_pages }, links: ListLinks { self_link: self_link.clone(), first: first.clone(), prev: prev.clone(), next: next.clone(), last: last.clone() },
            }),
            Input::Resource { data } => serialize(ResourceResponse { data: data.clone() }),
        };
        output
    }
}
