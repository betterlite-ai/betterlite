use betterlite_env::EnvError;
use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use rust_decimal::Decimal;

use crate::loader::{ConfigError, EnvironmentLookup};

mod tests;

pub const REPLICATE_PRICE_ENV: &str = "DEFAULT_REPLICATE_GPU_PRICE_PER_SECOND";

const DEFAULT_PRICE: Decimal = Decimal::from_parts(14, 0, 0, false, 4);

// Empty text is unset. Only plain signed decimals pass, so nan, inf and separators are errors.
pub fn decode_replicate_price(text: &str) -> Result<Option<Decimal>, ConfigError> {
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
            name: REPLICATE_PRICE_ENV.to_owned(),
        },
    })
}

pub fn load_replicate_gpu_price<L>(lookup: &L) -> Result<Decimal, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(text) = lookup.lookup(REPLICATE_PRICE_ENV) else {
        return Ok(DEFAULT_PRICE);
    };
    Ok(decode_replicate_price(&text)?.unwrap_or(DEFAULT_PRICE))
}
