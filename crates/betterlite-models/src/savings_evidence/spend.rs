use rust_decimal::Decimal;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SavingsSpend {
    pub compression: Decimal,
    pub prompt_caching: Decimal,
    pub autorouter: Decimal,
    pub gateway_injected_caching: Decimal,
}

// savings is baseline minus actual, unclamped: a worse baseline than the actual spend is a negative saving.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BaselineCosts {
    pub actual: Decimal,
    pub baseline: Decimal,
}

mod tests;
