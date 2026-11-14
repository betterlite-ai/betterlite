use betterlite_config::sections::studio_identity::Settings;

use super::request::AgentParams;
use crate::studio_files::resolve_base;
use crate::studio_identity::API_VERSION;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentTarget {
    pub url: String,
    pub query: Vec<(String, String)>,
}

#[must_use]
pub fn collection_target(params: &AgentParams, settings: &Settings) -> AgentTarget {
    let base = resolve_base(params.api_base.as_deref(), settings);
    AgentTarget {
        url: format!("{base}/{API_VERSION}/agents"),
        query: pagination(params),
    }
}

#[must_use]
pub fn item_target(name: &str, params: &AgentParams, settings: &Settings) -> AgentTarget {
    let base = resolve_base(params.api_base.as_deref(), settings);
    AgentTarget {
        url: format!("{base}/{API_VERSION}/agents/{name}"),
        query: Vec::new(),
    }
}

#[must_use]
pub fn versions_target(name: &str, params: &AgentParams, settings: &Settings) -> AgentTarget {
    let item = item_target(name, params, settings);
    AgentTarget {
        url: format!("{}/versions", item.url),
        query: pagination(params),
    }
}

fn pagination(params: &AgentParams) -> Vec<(String, String)> {
    let size = params
        .page_size
        .filter(|size| *size != 0)
        .map(|size| ("pageSize".to_owned(), size.to_string()));
    let token = params
        .page_token
        .clone()
        .filter(|token| !token.is_empty())
        .map(|token| ("pageToken".to_owned(), token));
    size.into_iter().chain(token).collect()
}
