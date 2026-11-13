use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use betterlite_models::tiered_pricing::Scalar;
use betterlite_models::tool_fees::error::LookupFailed;
use betterlite_models::tool_fees::model_info::{Rate, SearchContextRates, ToolModelInfo};
use betterlite_models::tool_fees::ports::ToolPriceSource;
use serde::Deserialize;

use super::lookup::CatalogLookup;

// A price key keeps whether it is missing, null or a number; any other shape reads as missing.
type Cell = Option<Option<Scalar>>;

fn rate(cell: Cell) -> Rate {
    match cell {
        None => Rate::Absent,
        Some(None) => Rate::Null,
        Some(Some(Scalar::Number(number))) => {
            parse_decimal_lexeme(&number.to_string()).map_or(Rate::Absent, Rate::Value)
        }
        Some(Some(_)) => Rate::Absent,
    }
}

#[derive(Deserialize, Default)]
struct ContextRates {
    #[serde(default, with = "serde_with::rust::double_option")]
    search_context_size_low: Cell,
    #[serde(default, with = "serde_with::rust::double_option")]
    search_context_size_medium: Cell,
    #[serde(default, with = "serde_with::rust::double_option")]
    search_context_size_high: Cell,
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
        let entry: Entry = serde_json::from_str(raw.get()).map_err(|_| LookupFailed)?;
        let context = entry.search_context_cost_per_query.unwrap_or_default();
        Ok(ToolModelInfo {
            litellm_provider: entry.litellm_provider,
            search_context: SearchContextRates {
                low: rate(context.search_context_size_low),
                medium: rate(context.search_context_size_medium),
                high: rate(context.search_context_size_high),
            },
            file_search_per_gb_per_day: rate(entry.file_search_gb_day),
            file_search_per_1k_calls: rate(entry.file_search_cost_per_1k_calls),
            vector_store_per_gb_per_day: rate(entry.vector_store_gb_day),
            computer_input_per_1k: rate(entry.computer_input_1k),
            computer_output_per_1k: rate(entry.computer_output_1k),
            code_interpreter_per_session: rate(entry.code_session),
        })
    }

    fn container_session_rate(&self, provider: &str) -> Result<Rate, LookupFailed> {
        let container = format!("{provider}/container");
        self.model_info(&container, None)
            .map(|info| info.code_interpreter_per_session)
    }
}
