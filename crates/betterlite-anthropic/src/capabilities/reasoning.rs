use betterlite_error::error::details::ResponseDetails;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_models::decisions::Json;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use indexmap::IndexMap;

use super::{MIN_THINKING_BUDGET_TOKENS, ThinkingBudgets};
use crate::anthropic_capability_catalog::CatalogSources;
use crate::anthropic_capability_catalog::capability::supports_model_capability;

pub struct ReasoningRequest<'request> {
    pub reasoning_effort: Option<&'request str>,
    pub model: &'request str,
    pub custom_llm_provider: &'request str,
    pub llm_provider: &'request str,
    pub budgets: &'request ThinkingBudgets,
}

#[derive(Debug, Eq, PartialEq, thiserror::Error)]
pub enum ReasoningEffortError {
    #[error(
        "Unmapped reasoning effort: {value}. Must be one of: 'minimal', 'low', 'medium', 'high', 'xhigh', 'max', 'none'."
    )]
    Unmapped {
        value: String,
        model: String,
        provider: String,
    },
}

impl ReasoningEffortError {
    #[must_use]
    pub fn classify(&self) -> ErrorClass {
        match self {
            Self::Unmapped {
                model, provider, ..
            } => {
                let context = ErrorContext {
                    message: self.to_string(),
                    model: Some(model.clone()),
                    provider: Some(provider.clone()),
                    max_retries: None,
                    num_retries: None,
                    body_type: None,
                    body_param: None,
                    body_code: None,
                };
                ErrorClass::BadRequest(Box::new(ResponseDetails::from(context)))
            }
        }
    }
}

// None and "none" remove thinking; an adaptive model answers before the string is validated.
pub async fn map_reasoning_effort<Source, Transport>(
    request: &ReasoningRequest<'_>,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
    catalogs: &CatalogSources<'_>,
) -> Result<Option<Json>, ReasoningEffortError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    let Some(effort) = request.reasoning_effort else {
        return Ok(None);
    };
    if effort == "none" {
        return Ok(None);
    }
    if supports_model_capability(
        request.model,
        "supports_adaptive_thinking",
        request.custom_llm_provider,
        environment,
        catalogs,
    )
    .await
    {
        return Ok(Some(adaptive_thinking()));
    }
    let budget = match effort {
        "low" => request.budgets.low,
        "medium" => request.budgets.medium,
        "high" => request.budgets.high,
        "xhigh" => request.budgets.xhigh,
        "max" => request.budgets.max,
        "minimal" => request.budgets.minimal.max(MIN_THINKING_BUDGET_TOKENS),
        _ => {
            return Err(ReasoningEffortError::Unmapped {
                value: effort.to_owned(),
                model: request.model.to_owned(),
                provider: request.llm_provider.to_owned(),
            });
        }
    };
    Ok(Some(enabled_thinking(budget)))
}

// Without display Anthropic defaults adaptive thinking to omitted and returns a blank thinking block.
fn adaptive_thinking() -> Json {
    let mut fields = IndexMap::new();
    fields.insert("type".to_owned(), Json::Text("adaptive".to_owned()));
    fields.insert("display".to_owned(), Json::Text("summarized".to_owned()));
    Json::Object(fields)
}

fn enabled_thinking(budget: i64) -> Json {
    let mut fields = IndexMap::new();
    fields.insert("type".to_owned(), Json::Text("enabled".to_owned()));
    fields.insert(
        "budget_tokens".to_owned(),
        Json::Number(serde_json::Number::from(budget)),
    );
    Json::Object(fields)
}
