use rust_decimal::Decimal;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingSpendIncrement {
    pub counter_key: String,
    pub increment: Decimal,
}
