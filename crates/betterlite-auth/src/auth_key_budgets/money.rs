use betterlite_db::budget_permission_rows::JsonData;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;

use crate::budget_counter::coercion::FloatInput as _;

// LiteLLM enforces only math.isfinite values; a parser that needs a nonfinite one reads the Python-repr spelling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Money {
    Finite(Decimal),
    Nan,
    PositiveInfinity,
    NegativeInfinity,
}

impl Money {
    #[must_use]
    pub fn from_f64(value: f64) -> Self {
        if value.is_nan() {
            return Self::Nan;
        }
        if value.is_infinite() {
            return Self::infinity(value.is_sign_negative());
        }
        Decimal::from_f64(value)
            .map_or_else(|| Self::infinity(value.is_sign_negative()), Self::Finite)
    }

    // a JSON number or a Python nonfinite spelling; a numeric string is not the declared type
    #[must_use]
    pub fn from_json(value: &JsonData) -> Option<Self> {
        match value {
            JsonData::Number(number) => Self::from_text(&number.to_string()),
            JsonData::String(text) => Self::spelled(text),
            JsonData::Null | JsonData::Bool(_) | JsonData::Array(_) | JsonData::Object(_) => None,
        }
    }

    #[must_use]
    pub fn from_text(text: &str) -> Option<Self> {
        Self::spelled(text)
            .or_else(|| text.decimal_value().map(Self::Finite))
            // Python's float() reads a finite spelling past Decimal's range as an infinity
            .or_else(|| text.parse::<f64>().ok().map(Self::from_f64))
    }

    #[must_use]
    pub const fn finite_value(self) -> Option<Decimal> {
        match self {
            Self::Finite(value) => Some(value),
            Self::Nan | Self::PositiveInfinity | Self::NegativeInfinity => None,
        }
    }

    // Python's >= where the left side is the finite spend a counter returned
    #[must_use]
    pub fn spend_at_least(spend: Decimal, budget: Self) -> bool {
        match budget {
            Self::Finite(limit) => spend >= limit,
            Self::Nan | Self::PositiveInfinity => false,
            Self::NegativeInfinity => true,
        }
    }

    // Python's < where the left side is the finite spend a counter returned
    #[must_use]
    pub fn spend_below(spend: Decimal, budget: Self) -> bool {
        match budget {
            Self::Finite(limit) => spend < limit,
            Self::Nan | Self::NegativeInfinity => false,
            Self::PositiveInfinity => true,
        }
    }

    #[must_use]
    pub fn plus(self, other: Self) -> Self {
        if let (Self::Finite(left), Self::Finite(right)) = (self, other) {
            return left
                .checked_add(right)
                .map_or_else(|| Self::infinity(left.is_sign_negative()), Self::Finite);
        }
        if self == Self::Nan || other == Self::Nan {
            return Self::Nan;
        }
        if self == other {
            return self;
        }
        if (self == Self::PositiveInfinity || other == Self::PositiveInfinity)
            && (self == Self::NegativeInfinity || other == Self::NegativeInfinity)
        {
            return Self::Nan;
        }
        if self == Self::PositiveInfinity || other == Self::PositiveInfinity {
            return Self::PositiveInfinity;
        }
        Self::NegativeInfinity
    }

    #[must_use]
    pub fn times(self, other: Self) -> Self {
        match (self, other) {
            (Self::Finite(left), Self::Finite(right)) => left.checked_mul(right).map_or_else(
                || Self::infinity(left.is_sign_negative() != right.is_sign_negative()),
                Self::Finite,
            ),
            (Self::Nan, _) | (_, Self::Nan) => Self::Nan,
            (Self::PositiveInfinity, Self::PositiveInfinity)
            | (Self::NegativeInfinity, Self::NegativeInfinity) => Self::PositiveInfinity,
            (Self::PositiveInfinity, Self::NegativeInfinity)
            | (Self::NegativeInfinity, Self::PositiveInfinity) => Self::NegativeInfinity,
            (Self::PositiveInfinity, Self::Finite(value))
            | (Self::Finite(value), Self::PositiveInfinity) => Self::scaled_infinity(value, false),
            (Self::NegativeInfinity, Self::Finite(value))
            | (Self::Finite(value), Self::NegativeInfinity) => Self::scaled_infinity(value, true),
        }
    }

    // max_budget * (percent / 100.0), with the percentage kept as a decimal fraction
    #[must_use]
    pub fn mul_percent(self, percent: i64) -> Self {
        Decimal::from(percent)
            .checked_div(Decimal::ONE_HUNDRED)
            .map_or(Self::Nan, |ratio| self.times(Self::Finite(ratio)))
    }

    // the same scaling for a threshold read from a digit-only map key
    #[must_use]
    pub fn mul_decimal_percent(self, percent: f64) -> Self {
        Decimal::from_f64(percent).map_or_else(
            || self.times(Self::from_f64(percent)),
            |value| {
                value
                    .checked_div(Decimal::ONE_HUNDRED)
                    .map_or(Self::Nan, |ratio| self.times(Self::Finite(ratio)))
            },
        )
    }

    pub(super) fn spelled(text: &str) -> Option<Self> {
        match text {
            "NaN" => Some(Self::Nan),
            "Infinity" => Some(Self::PositiveInfinity),
            "-Infinity" => Some(Self::NegativeInfinity),
            _ => None,
        }
    }

    const fn infinity(negative: bool) -> Self {
        if negative {
            Self::NegativeInfinity
        } else {
            Self::PositiveInfinity
        }
    }

    // 0.0 * inf is NaN; otherwise the product infinity takes the operands' sign product
    const fn scaled_infinity(value: Decimal, negative: bool) -> Self {
        if value.is_zero() {
            Self::Nan
        } else if value.is_sign_negative() != negative {
            Self::NegativeInfinity
        } else {
            Self::PositiveInfinity
        }
    }
}
