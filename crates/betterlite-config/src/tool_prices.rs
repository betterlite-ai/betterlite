use betterlite_env::EnvError;
use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use betterlite_models::tool_fees::prices::ToolPrices;
use rust_decimal::Decimal;

use crate::loader::{ConfigError, EnvironmentLookup};

mod tests;

pub const OPENAI_FILE_SEARCH_ENV: &str = "OPENAI_FILE_SEARCH_COST_PER_1K_CALLS";
pub const AZURE_FILE_SEARCH_ENV: &str = "AZURE_FILE_SEARCH_COST_PER_GB_PER_DAY";
pub const AZURE_VECTOR_STORE_ENV: &str = "AZURE_VECTOR_STORE_COST_PER_GB_PER_DAY";
pub const AZURE_COMPUTER_INPUT_ENV: &str = "AZURE_COMPUTER_USE_INPUT_COST_PER_1K_TOKENS";
pub const AZURE_COMPUTER_OUTPUT_ENV: &str = "AZURE_COMPUTER_USE_OUTPUT_COST_PER_1K_TOKENS";

// Empty text is unset. Only plain signed decimals pass, so nan, inf and separators are errors.
pub fn decode_tool_price(name: &str, text: &str) -> Result<Option<Decimal>, ConfigError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let plain = trimmed
        .bytes()
        .all(|byte| byte.is_ascii_digit() || matches!(byte, b'+' | b'-' | b'.' | b'e' | b'E'));
    if plain && let Ok(price) = parse_decimal_lexeme(trimmed) {
        return Ok(Some(price));
    }
    Err(ConfigError::Environment {
        source: EnvError::Number {
            name: name.to_owned(),
        },
    })
}

pub fn load_tool_prices<L>(lookup: &L) -> Result<ToolPrices, ConfigError>
where
    L: EnvironmentLookup,
{
    let pinned = ToolPrices::PINNED;
    Ok(ToolPrices {
        openai_search_per_1k: env_price(
            lookup,
            OPENAI_FILE_SEARCH_ENV,
            pinned.openai_search_per_1k,
        )?,
        azure_search_per_gb_day: env_price(
            lookup,
            AZURE_FILE_SEARCH_ENV,
            pinned.azure_search_per_gb_day,
        )?,
        azure_vector_per_gb_day: env_price(
            lookup,
            AZURE_VECTOR_STORE_ENV,
            pinned.azure_vector_per_gb_day,
        )?,
        azure_computer_input_per_1k: env_price(
            lookup,
            AZURE_COMPUTER_INPUT_ENV,
            pinned.azure_computer_input_per_1k,
        )?,
        azure_computer_output_per_1k: env_price(
            lookup,
            AZURE_COMPUTER_OUTPUT_ENV,
            pinned.azure_computer_output_per_1k,
        )?,
    })
}

fn env_price<L>(lookup: &L, name: &str, default: Decimal) -> Result<Decimal, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(text) = lookup.lookup(name) else {
        return Ok(default);
    };
    Ok(decode_tool_price(name, &text)?.unwrap_or(default))
}
