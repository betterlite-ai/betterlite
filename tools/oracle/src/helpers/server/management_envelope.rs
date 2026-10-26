use betterlite_server::management_envelopes::{
    FacetListResponse, ListResponse, PageLinks, PageMeta, ProblemDetail, ProblemDocument,
    ResourceResponse,
};
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Problem {
        problem: ProblemDetail,
        #[serde(default)]
        document: bool,
    },
    Facet {
        data: Vec<String>,
        meta: PageMeta,
        links: PageLinks,
    },
    List {
        data: Vec<Value>,
        meta: betterlite_server::management_envelopes::ListMeta,
        links: betterlite_server::management_envelopes::ListLinks,
    },
    Resource {
        data: Value,
    },
}

pub(crate) struct Op;

#[derive(Serialize)]
#[serde(untagged)]
enum Output {
    Problem(ProblemDetail),
    ProblemDocument(ProblemDocument),
    Facet(FacetListResponse),
    List(ListResponse<Value>),
    Resource(ResourceResponse<Value>),
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let output = match input {
            Input::Problem { problem, document } => match document {
                true => Output::ProblemDocument(ProblemDocument::from(problem)),
                false => Output::Problem(problem.clone()),
            },
            Input::Facet { data, meta, links } => Output::Facet(FacetListResponse {
                data: data.clone(),
                meta: meta.clone(),
                links: links.clone(),
            }),
            Input::List { data, meta, links } => Output::List(ListResponse {
                data: data.clone(),
                meta: meta.clone(),
                links: links.clone(),
            }),
            Input::Resource { data } => Output::Resource(ResourceResponse { data: data.clone() }),
        };
        serde_json::to_value(output).map_err(|error| {
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
}
