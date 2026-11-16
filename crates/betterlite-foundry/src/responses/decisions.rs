use betterlite_http::user_url::httpx_url::HttpxUrl;
use betterlite_openai::decisions::transformation::provider_config::DecisionsProviderConfig;
use percent_encoding::percent_decode_str;

use crate::credentials::{AZURE_AI_API_BASE_VAR, AZURE_AI_API_KEY_VAR};

use super::path::{PathError, requoted_path};

const SYSTEMONE_ROUTE: &str = "/providers/microsoft/v1/systemone";
const PROJECT_PREFIX: &str = "/api/projects/";
const ROUTE_SUFFIXES: [&str; 4] = ["/openai/v1", "/openai", "/models", "/v1"];

pub struct AzureAiDecisionsConfig;

impl DecisionsProviderConfig for AzureAiDecisionsConfig {
    const PATH: &'static str = SYSTEMONE_ROUTE;
    const API_KEY_ENV: &'static [&'static str] = &[AZURE_AI_API_KEY_VAR];
    const API_BASE_ENV: &'static [&'static str] = &[AZURE_AI_API_BASE_VAR];

    fn missing_api_base_message(&self, _custom_llm_provider: &str) -> String {
        String::from(
            "Missing AZURE_AI_API_BASE - set AZURE_AI_API_BASE or pass api_base, e.g. https://<resource>.services.ai.azure.com",
        )
    }

    fn get_complete_url(&self, api_base: &str, _model: &str) -> String {
        // the trait cannot fail, so an unparsable base is left for the transport to reject
        decisions_url(api_base).unwrap_or_else(|_| api_base.to_owned())
    }
}

fn decisions_url(api_base: &str) -> Result<String, PathError> {
    let original = HttpxUrl::parse(api_base)?;
    let decoded = percent_decode_str(&original.path).decode_utf8_lossy();
    let trimmed = decoded.trim_end_matches('/');
    let resource = trimmed.get(..route_start(trimmed)).unwrap_or(trimmed);
    let path = format!("{resource}{SYSTEMONE_ROUTE}");
    let mut url = original;
    url.path = requoted_path(&url, &path)?;
    url.query = None;
    url.fragment = None;
    Ok(url.href())
}

fn route_start(path: &str) -> usize {
    let mut start = path.len();
    for (index, _letter) in path.char_indices() {
        if path.get(index..).is_some_and(route_tail) {
            start = index;
            break;
        }
    }
    start
}

fn route_tail(rest: &str) -> bool {
    if rest.is_empty() || ROUTE_SUFFIXES.contains(&rest) {
        return true;
    }
    project_remainder(rest).is_some_and(|after| after.is_empty() || ROUTE_SUFFIXES.contains(&after))
}

fn project_remainder(rest: &str) -> Option<&str> {
    let project = rest.strip_prefix(PROJECT_PREFIX)?;
    let end = project.find('/').unwrap_or(project.len());
    if end == 0 {
        return None;
    }
    project.get(end..)
}
