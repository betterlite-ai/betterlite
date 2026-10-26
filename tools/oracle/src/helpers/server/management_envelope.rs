use betterlite_server::management_envelopes::{
    FacetListResponse, ListLinks, ListMeta, ListResponse, PageLinks, PageMeta, ProblemDetail,
    ResourceResponse,
};
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Problem {
        r#type: String,
        title: String,
        status: u16,
        detail: String,
        #[serde(default)]
        allowed: Option<Vec<String>>,
        #[serde(default)]
        document: bool,
    },
    Facet {
        data: Vec<String>,
        page: u64,
        page_size: u64,
        has_more: bool,
        #[serde(rename = "self", alias = "self_link")]
        self_link: String,
        #[serde(default)]
        prev: Option<String>,
        #[serde(default)]
        next: Option<String>,
    },
    List {
        data: Vec<Value>,
        total_count: u64,
        page: u64,
        page_size: u64,
        total_pages: u64,
        #[serde(rename = "self", alias = "self_link")]
        self_link: String,
        first: String,
        #[serde(default)]
        prev: Option<String>,
        #[serde(default)]
        next: Option<String>,
        last: String,
    },
    Resource {
        data: Value,
    },
}

pub(crate) struct Op;

#[derive(Serialize)]
struct ProblemDocument {
    r#type: String,
    title: String,
    status: u16,
    detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    allowed: Option<Vec<String>>,
}

fn serialization_error(error: &serde_json::Error) -> Box<crate::helpers::HelperError> {
    let message = error.to_string();
    Box::new(crate::helpers::HelperError {
        class: "SerializationError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn encode_response<T>(response: T) -> crate::helpers::HelperResult
where
    T: Serialize,
{
    let encoded = serde_json::to_value(response);
    encoded.map_err(|error| serialization_error(&error))
}

macro_rules! management_output {
    (problem $problem_type:expr, $title:expr, $status:expr, $detail:expr, $allowed:expr, $document:expr) => {
        if *$document {
            encode_response(ProblemDocument {
                r#type: $problem_type.clone(),
                title: $title.clone(),
                status: *$status,
                detail: $detail.clone(),
                allowed: $allowed.clone(),
            })
        } else {
            encode_response(ProblemDetail {
                r#type: $problem_type.clone(),
                title: $title.clone(),
                status: *$status,
                detail: $detail.clone(),
                allowed: $allowed.clone(),
            })
        }
    };
    (facet $data:expr, $page:expr, $page_size:expr, $has_more:expr, $self_link:expr, $prev:expr, $next:expr) => {
        encode_response(FacetListResponse {
            data: $data.clone(),
            meta: PageMeta {
                page: *$page,
                page_size: *$page_size,
                has_more: *$has_more,
            },
            links: PageLinks {
                self_link: $self_link.clone(),
                prev: $prev.clone(),
                next: $next.clone(),
            },
        })
    };
    (list $data:expr, $total_count:expr, $page:expr, $page_size:expr, $total_pages:expr, $self_link:expr, $first:expr, $prev:expr, $next:expr, $last:expr) => {
        encode_response(ListResponse {
            data: $data.clone(),
            meta: ListMeta {
                total_count: *$total_count,
                page: *$page,
                page_size: *$page_size,
                total_pages: *$total_pages,
            },
            links: ListLinks {
                self_link: $self_link.clone(),
                first: $first.clone(),
                prev: $prev.clone(),
                next: $next.clone(),
                last: $last.clone(),
            },
        })
    };
    (resource $data:expr) => {
        encode_response(ResourceResponse {
            data: $data.clone(),
        })
    };
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::Problem {
                r#type,
                title,
                status,
                detail,
                allowed,
                document,
            } => management_output!(problem r#type, title, status, detail, allowed, document),
            Input::Facet {
                data,
                page,
                page_size,
                has_more,
                self_link,
                prev,
                next,
            } => management_output!(facet data, page, page_size, has_more, self_link, prev, next),
            Input::List {
                data,
                total_count,
                page,
                page_size,
                total_pages,
                self_link,
                first,
                prev,
                next,
                last,
            } => {
                management_output!(list data, total_count, page, page_size, total_pages, self_link, first, prev, next, last)
            }
            Input::Resource { data } => management_output!(resource data),
        }
    }
}
