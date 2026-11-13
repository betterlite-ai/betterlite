pub mod error;
mod tests;

use betterlite_models::anthropic_pricing::search::web_search_cost as anthropic_web_search_cost;
use betterlite_models::cost_usage::server_tool_projection::ServerToolRecord;
use betterlite_models::cost_usage::tool_projection::ToolUsage;
use betterlite_models::price_fields::nested::SearchContextPricing;
use betterlite_models::price_lookup::PriceRecord;
use betterlite_models::tool_fees::model_info::{Rate, ToolModelInfo};
use betterlite_models::tool_fees::ports::ToolPricePolicy;
use rust_decimal::Decimal;

use error::FormulaError;

// The web-search formula a provider name selects in get_cost_for_web_search_request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchFormula {
    Gemini,
    Anthropic,
    VertexGemini,
    Xai,
    Groq,
}

// The arithmetic behind each routed formula; an error means no formula is installed for it or it failed.
pub trait GroundingFormulas {
    fn web_search(
        &self,
        formula: SearchFormula,
        usage: &ToolUsage,
        model_info: &ToolModelInfo,
    ) -> Result<Decimal, FormulaError>;

    fn google_maps(
        &self,
        requests: i64,
        model_info: &ToolModelInfo,
    ) -> Result<Decimal, FormulaError>;
}

// model_key stands in for the "key" entry LiteLLM keeps in model info, which ToolModelInfo does not carry; only the Vertex route reads it.
#[derive(Debug, Clone, Copy)]
pub struct GroundingPricing<'key, F> {
    pub formulas: F,
    pub model_key: Option<&'key str>,
}

impl<F> ToolPricePolicy for GroundingPricing<'_, F>
where
    F: GroundingFormulas,
{
    fn web_search_cost(
        &self,
        provider: &str,
        usage: &ToolUsage,
        model_info: &ToolModelInfo,
    ) -> Option<Decimal> {
        let formula = match provider {
            "gemini" => SearchFormula::Gemini,
            "anthropic" => SearchFormula::Anthropic,
            "perplexity" => return Some(Decimal::ZERO),
            "xai" => SearchFormula::Xai,
            "groq" => SearchFormula::Groq,
            vertex if vertex.starts_with("vertex_ai") => {
                if self
                    .model_key
                    .is_some_and(|key| key.to_lowercase().contains("claude"))
                {
                    SearchFormula::Anthropic
                } else {
                    SearchFormula::VertexGemini
                }
            }
            _ => return None,
        };
        self.formulas
            .web_search(formula, usage, model_info)
            .inspect_err(|error| tracing::warn!(%error, "no web search price"))
            .ok()
    }

    fn google_maps_cost(
        &self,
        provider: &str,
        requests: i64,
        model_info: &ToolModelInfo,
    ) -> Option<Decimal> {
        if provider != "gemini" && !provider.starts_with("vertex_ai") {
            return None;
        }
        self.formulas
            .google_maps(requests, model_info)
            .inspect_err(|error| tracing::warn!(%error, "no Google Maps price"))
            .ok()
    }
}

// Only the Anthropic arithmetic exists in the tree; the other formulas stay unavailable and say so.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProviderFormulas;

impl GroundingFormulas for ProviderFormulas {
    fn web_search(
        &self,
        formula: SearchFormula,
        usage: &ToolUsage,
        model_info: &ToolModelInfo,
    ) -> Result<Decimal, FormulaError> {
        if formula != SearchFormula::Anthropic {
            return Err(FormulaError::WebSearch(formula));
        }
        let medium = match model_info.search_context.medium {
            Rate::Value(rate) => Some(rate),
            Rate::Absent | Rate::Null => None,
        };
        let pricing = SearchContextPricing {
            search_context_size_low: None,
            search_context_size_medium: medium,
            search_context_size_high: None,
        };
        let record = usage.server_tool_use.as_ref().map(ServerToolRecord::Parsed);
        anthropic_web_search_cost(Some(&PriceRecord::new()), Some(&pricing), record)
            .map_err(|source| FormulaError::Anthropic { source })
    }

    fn google_maps(
        &self,
        _requests: i64,
        _model_info: &ToolModelInfo,
    ) -> Result<Decimal, FormulaError> {
        Err(FormulaError::GoogleMaps)
    }
}
