use std::collections::BTreeSet;

use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use crate::credentials::{
    ANTHROPIC_VERSION, BROWSER_ACCESS_HEADER, OAUTH_BETA_HEADER, OAUTH_BETA_HEADER_NAME,
    OAUTH_TOKEN_PREFIX, make_api_key_auth_header,
};
use crate::feature_flags::beta::{
    EFFORT_BETA, FILE_CODE_EXECUTION_BETA, FILES_API_BETA, MCP_CLIENT_BETA, OUTPUT_CONFIG_BETA,
    THINKING_DISPLAY_BETA, TOOL_CHANGES_BETA, computer_tool_beta_header,
};

use super::AnthropicHeaders;

pub const ANTHROPIC_VERSION_HEADER: &str = "anthropic-version";
pub const ACCEPT_HEADER: &str = "accept";
pub const CONTENT_TYPE_HEADER: &str = "content-type";
pub const AUTHORIZATION_HEADER: &str = "authorization";
pub const JSON_CONTENT_TYPE: &str = "application/json";
pub const TOOL_SEARCH_BETA: &str = "advanced-tool-use-2025-11-20";
pub const CODE_EXECUTION_BETA: &str = "code-execution-2025-08-25";
pub const SKILLS_BETA: &str = "skills-2025-10-02";
pub const WEB_SEARCH_BETA: &str = "web-search-2025-03-05";

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(rename_all = "snake_case")]
pub enum HeaderFeature {
    CodeExecution,
    Effort,
    FileId,
    InputExamples,
    McpServer,
    OutputConfig,
    Pdf,
    ProgrammaticToolCalling,
    PromptCaching,
    Skills,
    ThinkingDisplayUpdates,
    ToolChanges,
    ToolSearch,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HeaderFeatures {
    pub features: BTreeSet<HeaderFeature>,
    pub computer_tool_used: Option<String>,
    pub is_vertex_request: bool,
    pub web_search_tool_used: bool,
}

pub struct HeaderRequest<'request> {
    pub api_key: Option<&'request SecretString>,
    pub auth_token: Option<&'request SecretString>,
    pub anthropic_version: Option<&'request str>,
    pub api_base: Option<&'request str>,
    pub use_bearer_for_custom_base: bool,
    pub user_betas: Option<&'request [String]>,
    pub features: &'request HeaderFeatures,
}

#[must_use]
pub fn get_anthropic_headers(request: &HeaderRequest<'_>) -> AnthropicHeaders {
    let mut betas = derived_betas(request.features);
    let mut headers = IndexMap::new();
    headers.insert(
        ANTHROPIC_VERSION_HEADER.to_owned(),
        request
            .anthropic_version
            .unwrap_or(ANTHROPIC_VERSION)
            .to_owned(),
    );
    headers.insert(ACCEPT_HEADER.to_owned(), JSON_CONTENT_TYPE.to_owned());
    headers.insert(CONTENT_TYPE_HEADER.to_owned(), JSON_CONTENT_TYPE.to_owned());
    insert_auth_headers(&mut headers, &mut betas, request);
    if let Some(user_betas) = request.user_betas {
        betas.extend(user_betas.iter().cloned());
    }
    let beta_header = if request.features.is_vertex_request {
        // Vertex rejects every other beta, and needs this one only for web search
        request
            .features
            .web_search_tool_used
            .then(|| WEB_SEARCH_BETA.to_owned())
    } else {
        (!betas.is_empty()).then(|| betas.into_iter().collect::<Vec<_>>().join(","))
    };
    if let Some(value) = beta_header {
        headers.insert(OAUTH_BETA_HEADER_NAME.to_owned(), value);
    }
    AnthropicHeaders { values: headers }
}

fn insert_auth_headers(
    headers: &mut IndexMap<String, String>,
    betas: &mut BTreeSet<String>,
    request: &HeaderRequest<'_>,
) {
    let oauth = request
        .api_key
        .filter(|key| key.expose_secret().starts_with(OAUTH_TOKEN_PREFIX));
    if let Some(oauth_key) = oauth {
        let value = format!("Bearer {}", oauth_key.expose_secret());
        headers.insert(AUTHORIZATION_HEADER.to_owned(), value);
        headers.insert(BROWSER_ACCESS_HEADER.to_owned(), "true".to_owned());
        betas.insert(OAUTH_BETA_HEADER.to_owned());
    } else if let (None, Some(token)) = (request.api_key, request.auth_token) {
        let value = format!("Bearer {}", token.expose_secret());
        headers.insert(AUTHORIZATION_HEADER.to_owned(), value);
    } else if let Some(api_key) = request.api_key {
        let auth = make_api_key_auth_header(
            api_key,
            request.api_base,
            request.use_bearer_for_custom_base,
        );
        headers.insert(auth.name.to_owned(), auth.value.expose_secret().to_owned());
    }
}

fn derived_betas(features: &HeaderFeatures) -> BTreeSet<String> {
    let mut betas = BTreeSet::new();
    if let Some(version) = features
        .computer_tool_used
        .as_deref()
        .filter(|version| !version.is_empty())
    {
        betas.insert(computer_tool_beta_header(version).to_owned());
    }
    for feature in &features.features {
        insert_feature_betas(&mut betas, *feature);
    }
    betas
}

fn insert_feature_betas(betas: &mut BTreeSet<String>, feature: HeaderFeature) {
    match feature {
        HeaderFeature::CodeExecution => {
            betas.insert(CODE_EXECUTION_BETA.to_owned());
        }
        HeaderFeature::Effort => {
            betas.insert(EFFORT_BETA.to_owned());
        }
        HeaderFeature::FileId => {
            betas.insert(FILES_API_BETA.to_owned());
            betas.insert(FILE_CODE_EXECUTION_BETA.to_owned());
        }
        // prompt caching and PDFs work without a beta header, and LiteLLM keeps the flags only for compatibility
        HeaderFeature::Pdf | HeaderFeature::PromptCaching => {}
        HeaderFeature::InputExamples
        | HeaderFeature::ProgrammaticToolCalling
        | HeaderFeature::ToolSearch => {
            betas.insert(TOOL_SEARCH_BETA.to_owned());
        }
        HeaderFeature::McpServer => {
            betas.insert(MCP_CLIENT_BETA.to_owned());
        }
        HeaderFeature::OutputConfig => {
            betas.insert(OUTPUT_CONFIG_BETA.to_owned());
        }
        HeaderFeature::Skills => {
            betas.insert(SKILLS_BETA.to_owned());
        }
        HeaderFeature::ThinkingDisplayUpdates => {
            betas.insert(THINKING_DISPLAY_BETA.to_owned());
        }
        HeaderFeature::ToolChanges => {
            betas.insert(TOOL_CHANGES_BETA.to_owned());
        }
    }
}
