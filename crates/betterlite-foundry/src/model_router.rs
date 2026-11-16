use std::convert::Infallible;

use betterlite_config::sections::foundry_credentials::Settings;
use betterlite_context::chat_config_selection::foundry::{AzureAiRoute, get_azure_ai_route};
use betterlite_models::available_models::listing::ListingCredentials;
use betterlite_models::provider_metadata_factory::adapters::AzureFoundryModelInfo;
use betterlite_openai::chat::response::ChatResponse;
use betterlite_openai::openai_base_config::Object;
use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::message_content::Message;
use thiserror::Error;

use crate::anthropic_count::counter::AzureAnthropicTokenCounter;
use crate::anthropic_count::handler::FoundryCountHandler;
use crate::chat::request::RequestError;
use crate::chat::response::ResponseError;

mod tests;

pub const SELECTED_MODEL_KEY: &str = "azure_model_router_selected_model";

const MODEL_ROUTER_PREFIX: &str = "model_router/";
const MODEL_ROUTER_SEPARATOR_PREFIX: &str = "model-router/";

#[derive(Debug, Error)]
pub enum ModelRouterError {
    #[error("Azure Foundry does not support environment validation")]
    ValidationUnsupported,
}

pub struct RoutedChatResponse {
    pub response: ChatResponse,
    pub hidden_params: Object,
}

// LiteLLM's get_base_model: the parent request and response transforms receive the stripped name
#[must_use]
pub fn strip_model_router_prefix(model: &str) -> &str {
    if let Some((_, selected)) = model.split_once(MODEL_ROUTER_PREFIX) {
        return selected;
    }
    if let Some((_, selected)) = model.split_once(MODEL_ROUTER_SEPARATOR_PREFIX) {
        return selected;
    }
    model
}

pub fn transform_request(
    model: &str,
    messages: &[Message],
    optional_params: &Object,
) -> Result<Object, RequestError> {
    crate::chat::request::transform_request(
        strip_model_router_prefix(model),
        messages,
        optional_params,
    )
}

pub fn transform_response(
    model: &str,
    body: Json,
    existing_hidden: &Object,
) -> Result<RoutedChatResponse, ResponseError> {
    let base_model = strip_model_router_prefix(model);
    let response = crate::chat::response::transform_response(base_model, body)?;
    // get_hidden_params_dict reads the response's hidden map, which the typed response hands in here
    let mut hidden = existing_hidden.clone();
    insert_selected_model(&mut hidden, &response.model);
    Ok(RoutedChatResponse {
        response,
        hidden_params: hidden,
    })
}

#[must_use]
pub fn selected_model(hidden: Option<&Object>) -> Option<&str> {
    let params = hidden?;
    let selected = params.get(SELECTED_MODEL_KEY)?.as_str()?;
    if selected.is_empty() {
        return None;
    }
    Some(selected)
}

#[must_use]
pub fn is_model_router_call(model: Option<&str>, hidden: Option<&Object>) -> bool {
    if selected_model(hidden).is_some() {
        return true;
    }
    let deployment = hidden
        .and_then(|params| {
            params
                .get("litellm_model_name")
                .filter(|value| !value.is_null())
        })
        .or_else(|| hidden.and_then(|params| params.get("model")))
        .and_then(|value| value.as_str());
    let deployment_is_router =
        deployment.is_some_and(|name| get_azure_ai_route(name) == AzureAiRoute::ModelRouter);
    let caller_is_router =
        model.is_some_and(|name| get_azure_ai_route(name) == AzureAiRoute::ModelRouter);
    deployment_is_router || caller_is_router
}

#[must_use]
pub fn get_api_version<'version>(
    explicit: Option<&'version str>,
    global: Option<&'version str>,
    environment: Option<&'version str>,
) -> Option<&'version str> {
    if let Some(version) = explicit.filter(|version| !version.is_empty()) {
        return Some(version);
    }
    global
        .filter(|version| !version.is_empty())
        .or_else(|| environment.filter(|version| !version.is_empty()))
}

#[must_use]
pub fn get_token_counter<T>(
    info: &AzureFoundryModelInfo,
    handler: FoundryCountHandler<T>,
    environment: Settings,
) -> Option<AzureAnthropicTokenCounter<T>> {
    let model = info.model.as_deref().unwrap_or_default();
    if !model.to_lowercase().contains("claude") {
        return None;
    }
    Some(AzureAnthropicTokenCounter {
        handler,
        environment_api_key: environment.api_key,
        environment_api_base: environment.api_base,
    })
}

// the list is empty on every input, so its replacement mutation equals the body
#[must_use]
#[cfg_attr(test, mutants::skip)]
pub fn get_models(credentials: ListingCredentials<'_>) -> Vec<String> {
    tracing::debug!(
        has_api_key = credentials.api_key.is_some(),
        has_api_base = credentials.api_base.is_some(),
        "Azure AI has no model listing endpoint, so get_models returns an empty list"
    );
    Vec::new()
}

pub fn validate_environment() -> Result<Infallible, ModelRouterError> {
    tracing::debug!("Azure Foundry does not support environment validation");
    Err(ModelRouterError::ValidationUnsupported)
}

fn insert_selected_model(hidden: &mut Object, selected_model: &str) {
    if selected_model.is_empty() {
        return;
    }
    hidden.insert(
        SELECTED_MODEL_KEY.to_owned(),
        serde_json::json!(selected_model),
    );
}
