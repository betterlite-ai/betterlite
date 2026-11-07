use rust_decimal::Decimal;

use crate::management_guards::error::ManagementGuardError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Amount {
    Finite(Decimal),
    Nan,
    PositiveInfinity,
    NegativeInfinity,
}

#[must_use]
pub fn describe_amount(value: Option<Amount>) -> String {
    match value {
        None => "null".to_owned(),
        Some(Amount::Finite(decimal)) => decimal.to_string(),
        Some(Amount::Nan) => "nan".to_owned(),
        Some(Amount::PositiveInfinity) => "inf".to_owned(),
        Some(Amount::NegativeInfinity) => "-inf".to_owned(),
    }
}

pub fn validate_finite_spend(spend: Option<Amount>) -> Result<(), ManagementGuardError> {
    match spend {
        // a non-finite spend never reaches Decimal, and a finite negative spend is payable
        Some(Amount::Nan | Amount::PositiveInfinity | Amount::NegativeInfinity) => {
            Err(ManagementGuardError::NonFiniteSpend {
                received: describe_amount(spend),
            })
        }
        Some(Amount::Finite(_)) | None => Ok(()),
    }
}
