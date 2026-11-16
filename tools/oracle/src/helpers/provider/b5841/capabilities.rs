use std::future::Future;

use betterlite_anthropic::anthropic_capability_catalog::OptionalParams;
use betterlite_anthropic::anthropic_capability_catalog::thinking::ThinkingError;
use betterlite_anthropic::capabilities::reasoning::ReasoningEffortError;
use betterlite_core::api::messages_native_transform::capabilities::{
    LegacyShapes, Thinking, ThinkingCapabilities,
};
use betterlite_models::decisions::Json;
use serde_json::Number;

// The cases use claude models, so the fake answers from the name and never reads a catalog
pub(super) struct FakeCapabilities;

impl ThinkingCapabilities for FakeCapabilities {
    fn supports(
        &mut self,
        _model: &str,
        _key: &str,
        _provider: &str,
    ) -> impl Future<Output = bool> + Send {
        async { false }
    }

    fn map_effort<'seam>(
        &'seam mut self,
        model: &'seam str,
        effort: &'seam str,
        provider: &'seam str,
    ) -> impl Future<Output = Result<Option<Thinking>, ReasoningEffortError>> + Send + 'seam {
        async move { effort_budget(model, effort, provider) }
    }

    fn validate_effort(
        &mut self,
        _model: &str,
        _effort: Option<&str>,
        _provider: &str,
    ) -> impl Future<Output = Option<String>> + Send {
        async { None }
    }

    fn supports_effort_param(
        &mut self,
        _model: &str,
        _provider: &str,
    ) -> impl Future<Output = bool> + Send {
        async { false }
    }

    fn drop_disabled_thinking(
        &mut self,
        _model: &str,
        _params: &mut OptionalParams,
        _provider: &str,
    ) -> impl Future<Output = bool> + Send {
        async { false }
    }

    fn translate_legacy_thinking<'seam>(
        &'seam mut self,
        _request: &'seam mut LegacyShapes<'seam>,
    ) -> impl Future<Output = Result<(), ThinkingError>> + Send + 'seam {
        async { Ok(()) }
    }
}

// _map_reasoning_effort's static budgets for a model that is not adaptive
fn effort_budget(
    model: &str,
    effort: &str,
    provider: &str,
) -> Result<Option<Thinking>, ReasoningEffortError> {
    let budget: i64 = match effort {
        "none" => return Ok(None),
        "minimal" | "low" => 1024,
        "medium" => 2048,
        "high" => 4096,
        "xhigh" => 8192,
        "max" => 16384,
        _ => {
            return Err(ReasoningEffortError::Unmapped {
                value: effort.to_owned(),
                model: model.to_owned(),
                provider: provider.to_owned(),
            });
        }
    };
    let mut thinking = Thinking::new();
    thinking.insert("type".to_owned(), Json::Text(String::from("enabled")));
    thinking.insert(
        "budget_tokens".to_owned(),
        Json::Number(Number::from(budget)),
    );
    Ok(Some(thinking))
}
