use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use betterlite_models::price_lookup::PriceLookup as _;
use betterlite_models::tiered_pricing::Scalar;
use betterlite_models::tool_fees::error::LookupFailed;
use betterlite_models::tool_fees::model_info::{Rate, SearchContextRates, ToolModelInfo};
use betterlite_models::tool_fees::ports::ToolPriceSource;
use serde::Deserialize;

use super::lookup::CatalogLookup;

const CODE_SESSION_KEY: &str = "code_interpreter_cost_per_session";

// A price key keeps whether it is missing, null or a number; any other shape reads as missing.
type Cell = Option<Option<Scalar>>;

fn scalar_rate(value: &Scalar) -> Rate {
    match value {
        Scalar::Number(number) => {
            parse_decimal_lexeme(&number.to_string()).map_or(Rate::Absent, Rate::Value)
        }
        Scalar::Null => Rate::Null,
        Scalar::Bool(_) | Scalar::Text(_) | Scalar::Array(_) | Scalar::Object(_) => Rate::Absent,
    }
}

fn rate(cell: Cell) -> Rate {
    match cell {
        Some(None) => Rate::Null,
        Some(Some(value)) => scalar_rate(&value),
        None => Rate::Absent,
    }
}

#[derive(Deserialize)]
struct ProjectedKey {
    key: String,
}

#[derive(Deserialize, Default)]
struct ContextRates {
    #[serde(
        default,
        rename = "search_context_size_low",
        with = "serde_with::rust::double_option"
    )]
    low: Cell,
    #[serde(
        default,
        rename = "search_context_size_medium",
        with = "serde_with::rust::double_option"
    )]
    medium: Cell,
    #[serde(
        default,
        rename = "search_context_size_high",
        with = "serde_with::rust::double_option"
    )]
    high: Cell,
}

#[derive(Deserialize)]
struct Entry {
    #[serde(default)]
    litellm_provider: Option<String>,
    #[serde(default)]
    search_context_cost_per_query: Option<ContextRates>,
    #[serde(
        default,
        rename = "file_search_cost_per_gb_per_day",
        with = "serde_with::rust::double_option"
    )]
    file_search_gb_day: Cell,
    #[serde(default, with = "serde_with::rust::double_option")]
    file_search_cost_per_1k_calls: Cell,
    #[serde(
        default,
        rename = "vector_store_cost_per_gb_per_day",
        with = "serde_with::rust::double_option"
    )]
    vector_store_gb_day: Cell,
    #[serde(
        default,
        rename = "computer_use_input_cost_per_1k_tokens",
        with = "serde_with::rust::double_option"
    )]
    computer_input_1k: Cell,
    #[serde(
        default,
        rename = "computer_use_output_cost_per_1k_tokens",
        with = "serde_with::rust::double_option"
    )]
    computer_output_1k: Cell,
    #[serde(
        default,
        rename = "code_interpreter_cost_per_session",
        with = "serde_with::rust::double_option"
    )]
    code_session: Cell,
}

impl ToolPriceSource for CatalogLookup {
    fn model_info(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Result<ToolModelInfo, LookupFailed> {
        let raw = self
            .resolved(model, custom_llm_provider)
            .ok_or(LookupFailed)?;
        let entry: Entry = serde_json::from_str(raw.get()).map_err(|_source| LookupFailed)?;
        let context = entry.search_context_cost_per_query.unwrap_or_default();
        Ok(ToolModelInfo {
            litellm_provider: entry.litellm_provider,
            search_context: SearchContextRates {
                low: rate(context.low),
                medium: rate(context.medium),
                high: rate(context.high),
            },
            file_search_per_gb_per_day: rate(entry.file_search_gb_day),
            file_search_per_1k_calls: rate(entry.file_search_cost_per_1k_calls),
            vector_store_per_gb_per_day: rate(entry.vector_store_gb_day),
            computer_input_per_1k: rate(entry.computer_input_1k),
            computer_output_per_1k: rate(entry.computer_output_1k),
            code_interpreter_per_session: rate(entry.code_session),
        })
    }

    // Ports _get_code_interpreter_cost_from_model_map: resolve the container entry, then read its raw rate.
    fn container_session_rate(&self, provider: &str) -> Result<Rate, LookupFailed> {
        let container = format!("{provider}/container");
        let projected = self
            .resolved(&container, Some(provider))
            .ok_or(LookupFailed)?;
        let located: ProjectedKey =
            serde_json::from_str(projected.get()).map_err(|_source| LookupFailed)?;
        let entry = self.raw_cost_map_entry(&located.key).ok_or(LookupFailed)?;
        Ok(entry
            .get(CODE_SESSION_KEY)
            .map_or(Rate::Absent, scalar_rate))
    }
}
