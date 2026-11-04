use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::Number;
use thiserror::Error;

#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum CostInput {
    Number(Number),
    Mapping { total_cost: Option<Number> },
    Null,
}

#[derive(Debug, Error)]
pub(super) enum CostError {
    #[error("cost is not a valid decimal")]
    Decimal(#[from] rust_decimal::Error),
}

impl TryFrom<CostInput> for Option<Decimal> {
    type Error = CostError;

    fn try_from(cost: CostInput) -> Result<Self, Self::Error> {
        let number = match cost {
            CostInput::Number(number) => Some(number),
            CostInput::Mapping { total_cost } => total_cost,
            CostInput::Null => None,
        };
        number
            .map(|amount| {
                let text = amount.to_string();
                Decimal::from_str_exact(&text)
                    .or_else(|_| Decimal::from_scientific(&text))
                    .map_err(CostError::from)
            })
            .transpose()
    }
}
