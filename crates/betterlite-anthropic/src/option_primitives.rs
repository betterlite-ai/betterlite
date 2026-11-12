use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::lookup::limits::max_output_tokens;
use betterlite_openai::base_helpers::provider_config;
use betterlite_openai::error::OpenAiError;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Number;

pub mod context_management;
mod tests;
pub mod user_id;

pub const CUSTOM_LLM_PROVIDER: &str = "anthropic";
pub const WEB_SEARCH_TOOL_TYPE: &str = "web_search_20250305";
pub const WEB_SEARCH_TOOL_NAME: &str = "web_search";

const APPROXIMATE_LOCATION_TYPE: &str = "approximate";

#[derive(Debug, thiserror::Error)]
pub enum OptionPrimitivesError {
    #[error(transparent)]
    Config(#[from] OpenAiError),
    #[error("the option config does not round-trip as JSON")]
    ConfigJson(#[source] serde_json::Error),
    #[error("a stop sequence is not text")]
    StopSequenceNotText,
    #[error("search context size {value:?} is not one of low, medium or high")]
    UnknownSearchContextSize { value: String },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct OptionDefaults {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_sequences: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<Number>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<Number>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<IndexMap<String, Json>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
}

impl OptionDefaults {
    pub fn get_config(
        &self,
        model: Option<&str>,
        catalog: &ModelCatalog,
        default_chat_max_tokens: i64,
    ) -> Result<Json, OptionPrimitivesError> {
        let mut config = provider_config(self)?;
        if !config.contains_key("max_tokens") {
            let max_tokens = get_max_tokens_for_model(catalog, model, default_chat_max_tokens);
            config.insert("max_tokens".to_owned(), serde_json::json!(max_tokens));
        }
        let projected = serde_json::to_value(config).map_err(OptionPrimitivesError::ConfigJson)?;
        serde_json::from_value(projected).map_err(OptionPrimitivesError::ConfigJson)
    }
}

#[must_use]
pub fn get_max_tokens_for_model(
    catalog: &ModelCatalog,
    model: Option<&str>,
    default_chat_max_tokens: i64,
) -> i64 {
    let Some(name) = model else {
        return default_chat_max_tokens;
    };
    match max_output_tokens(catalog, name) {
        Ok(Some(tokens)) => i64::try_from(tokens).unwrap_or(default_chat_max_tokens),
        Ok(None) | Err(_) => default_chat_max_tokens,
    }
}

#[must_use]
pub const fn resolved_provider(configured: Option<&str>) -> &str {
    match configured {
        Some(provider) if !provider.is_empty() => provider,
        Some(_) | None => CUSTOM_LLM_PROVIDER,
    }
}

pub fn map_stop_sequences(
    stop: &Json,
    drop_params: bool,
) -> Result<Option<Vec<String>>, OptionPrimitivesError> {
    match stop {
        Json::Text(text) => {
            if drop_params && is_whitespace_only(text) {
                return Ok(None);
            }
            Ok(Some(vec![text.clone()]))
        }
        Json::List(items) => {
            let mut kept = Vec::with_capacity(items.len());
            for item in items {
                let Json::Text(text) = item else {
                    return Err(OptionPrimitivesError::StopSequenceNotText);
                };
                if drop_params && is_whitespace_only(text) {
                    continue;
                }
                kept.push(text.clone());
            }
            Ok((!kept.is_empty()).then_some(kept))
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_) => Ok(None),
    }
}

fn is_whitespace_only(text: &str) -> bool {
    !text.is_empty()
        && text.chars().all(|character| {
            character.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&character)
        })
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct WebSearchOptions {
    pub search_context_size: Option<String>,
    pub user_location: Option<WebSearchUserLocation>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct WebSearchUserLocation {
    pub approximate: Option<ApproximateLocation>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ApproximateLocation {
    pub city: Option<String>,
    pub country: Option<String>,
    pub region: Option<String>,
    pub timezone: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WebSearchTool {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub name: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_location: Option<WebSearchLocation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_uses: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WebSearchLocation {
    #[serde(rename = "type")]
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub city: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchContextSize {
    Low,
    Medium,
    High,
}

impl SearchContextSize {
    const fn max_uses(self) -> u8 {
        match self {
            Self::Low => 1,
            Self::Medium => 5,
            Self::High => 10,
        }
    }
}

impl TryFrom<&str> for SearchContextSize {
    type Error = OptionPrimitivesError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "low" => Ok(Self::Low),
            "medium" => Ok(Self::Medium),
            "high" => Ok(Self::High),
            unknown => Err(OptionPrimitivesError::UnknownSearchContextSize {
                value: unknown.to_owned(),
            }),
        }
    }
}

pub fn map_web_search_tool(
    options: &WebSearchOptions,
) -> Result<WebSearchTool, OptionPrimitivesError> {
    let user_location = options.user_location.as_ref().and_then(|location| {
        let approximate = location.approximate.as_ref()?;
        Some(WebSearchLocation {
            kind: APPROXIMATE_LOCATION_TYPE,
            city: approximate.city.clone(),
            country: approximate.country.clone(),
            region: approximate.region.clone(),
            timezone: approximate.timezone.clone(),
        })
    });
    let max_uses = match options.search_context_size.as_deref() {
        None => None,
        Some(size) => Some(SearchContextSize::try_from(size)?.max_uses()),
    };
    Ok(WebSearchTool {
        kind: WEB_SEARCH_TOOL_TYPE,
        name: WEB_SEARCH_TOOL_NAME,
        user_location,
        max_uses,
    })
}
