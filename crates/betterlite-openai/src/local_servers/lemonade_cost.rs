use rust_decimal::Decimal;

// Lemonade is self-hosted, so LiteLLM's cost_per_token returns this pair for every model and usage
pub const COST_PER_TOKEN: (Decimal, Decimal) = (Decimal::ZERO, Decimal::ZERO);
