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

fn invalid_input() -> crate::helpers::HelperResult {
    let error = Box::new(crate::helpers::HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: "management envelope variant mismatch".to_owned(),
    });
    Err(error)
}

fn problem_output(input: &Input) -> crate::helpers::HelperResult {
    let Input::Problem {
        r#type,
        title,
        status,
        detail,
        allowed,
        document,
    } = input
    else {
        return invalid_input();
    };
    if *document {
        serde_json::to_value(ProblemDocument {
            r#type: r#type.clone(),
            title: title.clone(),
            status: *status,
            detail: detail.clone(),
            allowed: allowed.clone(),
        })
        .map_err(|error| serialization_error(&error))
    } else {
        serde_json::to_value(ProblemDetail {
            r#type: r#type.clone(),
            title: title.clone(),
            status: *status,
            detail: detail.clone(),
            allowed: allowed.clone(),
        })
        .map_err(|error| serialization_error(&error))
    }
}

fn facet_output(input: &Input) -> crate::helpers::HelperResult {
    let Input::Facet {
        data,
        page,
        page_size,
        has_more,
        self_link,
        prev,
        next,
    } = input
    else {
        return invalid_input();
    };
    serde_json::to_value(FacetListResponse {
        data: data.clone(),
        meta: PageMeta {
            page: *page,
            page_size: *page_size,
            has_more: *has_more,
        },
        links: PageLinks {
            self_link: self_link.clone(),
            prev: prev.clone(),
            next: next.clone(),
        },
    })
    .map_err(|error| serialization_error(&error))
}

fn list_output(input: &Input) -> crate::helpers::HelperResult {
    let Input::List {
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
    } = input
    else {
        return invalid_input();
    };
    serde_json::to_value(ListResponse {
        data: data.clone(),
        meta: ListMeta {
            total_count: *total_count,
            page: *page,
            page_size: *page_size,
            total_pages: *total_pages,
        },
        links: ListLinks {
            self_link: self_link.clone(),
            first: first.clone(),
            prev: prev.clone(),
            next: next.clone(),
            last: last.clone(),
        },
    })
    .map_err(|error| serialization_error(&error))
}

fn resource_output(input: &Input) -> crate::helpers::HelperResult {
    let Input::Resource { data } = input else {
        return invalid_input();
    };
    serde_json::to_value(ResourceResponse { data: data.clone() })
        .map_err(|error| serialization_error(&error))
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::Problem { .. } => problem_output(input),
            Input::Facet { .. } => facet_output(input),
            Input::List { .. } => list_output(input),
            Input::Resource { .. } => resource_output(input),
        }
    }
}
