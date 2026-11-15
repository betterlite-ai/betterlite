use rust_decimal::Decimal;

mod tests;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallFailure {
    pub error: String,
    pub cost: Decimal,
    pub classifier_cost: Decimal,
}

impl CallFailure {
    #[must_use]
    pub fn new(error: String) -> Self {
        Self {
            error,
            cost: Decimal::ZERO,
            classifier_cost: Decimal::ZERO,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShadowResponse {
    pub text: String,
    pub model: String,
    pub tier: Option<String>,
    pub cost: Decimal,
    pub classifier_cost: Decimal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgeVerdict {
    pub preference: String,
    pub confidence: Decimal,
    pub cost: Decimal,
}

impl JudgeVerdict {
    #[must_use]
    pub fn parsed(preference: String, confidence: Decimal, cost: Decimal) -> Self {
        Self {
            preference,
            confidence: confidence.clamp(Decimal::ZERO, Decimal::ONE),
            cost,
        }
    }
}
