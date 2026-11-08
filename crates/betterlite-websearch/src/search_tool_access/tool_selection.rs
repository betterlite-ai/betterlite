use std::fmt;

use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::value::RawValue;

pub type SearchToolParams = IndexMap<String, Box<RawValue>>;

#[derive(Clone, Deserialize)]
#[serde(try_from = "RawSearchToolConfig")]
pub struct SearchToolConfig {
    pub search_tool_name: Option<String>,
    pub search_provider: Option<String>,
    pub litellm_params: SearchToolParams,
}

impl fmt::Debug for SearchToolConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SearchToolConfig")
            .field("search_tool_name", &self.search_tool_name)
            .field("search_provider", &self.search_provider)
            .field(
                "litellm_params",
                &self.litellm_params.keys().collect::<Vec<_>>(),
            )
            .finish()
    }
}

#[derive(Deserialize)]
struct RawSearchToolConfig {
    #[serde(default)]
    search_tool_name: Option<String>,
    #[serde(default)]
    litellm_params: Option<SearchToolParams>,
}

#[derive(Debug, thiserror::Error)]
pub enum SearchToolConfigError {
    #[error("litellm_params.search_provider must be a string or null")]
    SearchProvider(#[source] serde_json::Error),
}

impl TryFrom<RawSearchToolConfig> for SearchToolConfig {
    type Error = SearchToolConfigError;

    fn try_from(raw: RawSearchToolConfig) -> Result<Self, Self::Error> {
        let mut litellm_params = raw.litellm_params.unwrap_or_default();
        let search_provider = litellm_params
            .shift_remove("search_provider")
            .map(|value| serde_json::from_str::<Option<String>>(value.get()))
            .transpose()
            .map_err(SearchToolConfigError::SearchProvider)?
            .flatten();
        Ok(Self {
            search_tool_name: raw.search_tool_name,
            search_provider,
            litellm_params,
        })
    }
}

#[derive(Debug, Clone)]
pub struct SearchToolSelector {
    search_tool_name: Option<String>,
}

impl SearchToolSelector {
    #[must_use]
    pub fn new(search_tool_name: Option<String>) -> Self {
        match search_tool_name {
            Some(name) if !name.is_empty() => Self {
                search_tool_name: Some(name),
            },
            Some(_) | None => Self {
                search_tool_name: None,
            },
        }
    }

    #[must_use]
    pub fn select_from_router<'tools>(
        &self,
        search_tools: Option<&'tools [SearchToolConfig]>,
    ) -> Option<&'tools SearchToolConfig> {
        let tools = search_tools?;
        if tools.is_empty() {
            return None;
        }
        self.select_from_list(tools)
    }

    #[must_use]
    pub fn select_from_list<'tools>(
        &self,
        search_tools: &'tools [SearchToolConfig],
    ) -> Option<&'tools SearchToolConfig> {
        if let Some(name) = self.search_tool_name.as_deref() {
            if let Some(found) = search_tools
                .iter()
                .find(|tool| tool.search_tool_name.as_deref() == Some(name))
            {
                tracing::debug!(
                    "WebSearchInterception: Found search tool '{}' from router with provider '{}'",
                    name,
                    found.search_provider.as_deref().unwrap_or("None")
                );
                return Some(found);
            }
            tracing::debug!(
                "WebSearchInterception: Search tool '{}' not found in router, falling back to first available or perplexity",
                name
            );
        }
        let first = search_tools.first()?;
        tracing::debug!(
            "WebSearchInterception: Using first available search tool from router with provider '{}'",
            first.search_provider.as_deref().unwrap_or("None")
        );
        Some(first)
    }
}

#[must_use]
pub fn selected_search_tool_name(search_tool: Option<&SearchToolConfig>) -> Option<&str> {
    let name = search_tool?.search_tool_name.as_deref()?;
    if name.is_empty() {
        return None;
    }
    Some(name)
}
