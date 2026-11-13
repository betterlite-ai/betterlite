use betterlite_config::sections::studio_identity::Settings;

use super::request::AgentParams;
use crate::studio_files::resolve_base;
use crate::studio_identity::API_VERSION;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentTarget {
    pub url: String,
    pub query: Vec<(String, String)>,
}

// get_complete_url and transform_list_request build the same collection URL; create sends no query.
pub fn collection_target(params: &AgentParams, settings: &Settings) -> AgentTarget {
    let base = resolve_base(params.api_base.as_deref(), settings);
    AgentTarget {
        url: format!("{base}/{API_VERSION}/agents"),
        query: pagination(params),
    }
}

pub fn item_target(name: &str, params: &AgentParams, settings: &Settings) -> AgentTarget {
    let base = resolve_base(params.api_base.as_deref(), settings);
    AgentTarget {
        url: format!("{base}/{API_VERSION}/agents/{name}"),
        query: Vec::new(),
    }
}

pub fn versions_target(name: &str, params: &AgentParams, settings: &Settings) -> AgentTarget {
    let base = resolve_base(params.api_base.as_deref(), settings);
    AgentTarget {
        url: format!("{base}/{API_VERSION}/agents/{name}/versions"),
        query: pagination(params),
    }
}

fn pagination(params: &AgentParams) -> Vec<(String, String)> {
    let mut query = Vec::new();
    if let Some(size) = params.page_size.filter(|size| *size != 0) {
        query.push(("pageSize".to_owned(), size.to_string()));
    }
    if let Some(token) = params
        .page_token
        .as_deref()
        .filter(|token| !token.is_empty())
    {
        query.push(("pageToken".to_owned(), token.to_owned()));
    }
    query
}
