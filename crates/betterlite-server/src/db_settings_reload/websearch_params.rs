use betterlite_config::agentic_loop_ceiling::{
    AgenticLoopCeilingError, AgenticLoopValue, validated_max_agentic_loops,
};
use betterlite_config::sections::settings::JsonValue;
use betterlite_config::sections::websearch_request_hooks::Settings as WebsearchParams;
use betterlite_openai::openai_base_config::Object as JsonMap;
use serde::Deserialize;
use serde::de::IgnoredAny;
use thiserror::Error;

const LOOP_FIELD: &str = "websearch_interception_params.max_agentic_loops";

#[derive(Debug, Error)]
pub enum WebsearchParamError {
    #[error("websearch_interception_params.enabled_providers must be a list of provider names")]
    EnabledProvider,
    #[error("websearch_interception_params.search_tool_name must be a string")]
    SearchToolName,
    #[error("websearch_interception_params is not a valid settings mapping")]
    Shape(#[source] serde_json::Error),
    #[error(transparent)]
    MaxAgenticLoops(#[from] AgenticLoopCeilingError),
}

// The declared WebSearchInterceptionConfig: only handler fields are modeled here, "enabled" gates the callback.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct StoredHandlerParams {
    enabled_providers: Option<StoredProviders>,
    search_tool_name: Option<StoredToolName>,
    max_agentic_loops: Option<StoredLoopNumber>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum StoredProviders {
    Names(Vec<String>),
    WrongItems(WrongProviderItems),
    NotAList(IgnoredAny),
}

#[derive(Debug, Deserialize)]
#[serde(transparent)]
struct WrongProviderItems {
    items: Vec<IgnoredAny>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum StoredToolName {
    Text(String),
    Wrong(IgnoredAny),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum StoredLoopNumber {
    Boolean(bool),
    Integer(i64),
    Float(f64),
    Text(String),
    Other(IgnoredAny),
}

// _websearch_handler_params: an enabled_providers that is not a non-empty list is dropped, so the handler's own defaults apply.
pub(crate) fn handler_params(stored: &JsonMap) -> Result<WebsearchParams, WebsearchParamError> {
    let parsed: StoredHandlerParams = serde_json::from_value(JsonValue::Object(stored.clone()))
        .map_err(WebsearchParamError::Shape)?;
    let enabled_providers = match parsed.enabled_providers {
        Some(StoredProviders::Names(names)) if !names.is_empty() => names,
        Some(StoredProviders::WrongItems(items)) => {
            drop(items.items);
            return Err(WebsearchParamError::EnabledProvider);
        }
        Some(StoredProviders::Names(_) | StoredProviders::NotAList(_)) | None => {
            WebsearchParams::default().enabled_providers
        }
    };
    let search_tool_name = match parsed.search_tool_name {
        Some(StoredToolName::Text(name)) => Some(name),
        Some(StoredToolName::Wrong(_)) => return Err(WebsearchParamError::SearchToolName),
        None => None,
    };
    let ceiling = match parsed.max_agentic_loops {
        Some(StoredLoopNumber::Boolean(flag)) => AgenticLoopValue::Boolean(flag),
        Some(StoredLoopNumber::Integer(number)) => AgenticLoopValue::Integer(number),
        Some(StoredLoopNumber::Float(number)) => AgenticLoopValue::Float(number),
        Some(StoredLoopNumber::Text(text)) => AgenticLoopValue::Text(text),
        Some(StoredLoopNumber::Other(_)) => {
            return Err(WebsearchParamError::MaxAgenticLoops(
                AgenticLoopCeilingError::NotWholeNumber {
                    field: LOOP_FIELD.to_owned(),
                },
            ));
        }
        None => AgenticLoopValue::Null,
    };
    Ok(WebsearchParams {
        enabled_providers,
        search_tool_name,
        max_agentic_loops: validated_max_agentic_loops(&ceiling, LOOP_FIELD)?,
    })
}
