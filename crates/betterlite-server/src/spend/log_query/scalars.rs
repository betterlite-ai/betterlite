use std::str::FromStr as _;

use rust_decimal::Decimal;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IntegerText {
    Value(i64),
    AboveSignedRange,
    NotAnInteger,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NumericIssue {
    NotAnInteger,
    NotABoolean,
    NotANumber,
    NotFinite,
    OutOfRange,
}

impl NumericIssue {
    pub(crate) const fn parts(self) -> (&'static str, &'static str) {
        match self {
            Self::NotAnInteger => (
                "int_parsing",
                "Input should be a valid integer, unable to parse string as an integer",
            ),
            Self::NotABoolean => (
                "bool_parsing",
                "Input should be a valid boolean, unable to interpret input",
            ),
            Self::NotANumber => (
                "float_parsing",
                "Input should be a valid number, unable to parse string as a number",
            ),
            Self::NotFinite => ("finite_number", "Input should be a finite number"),
            Self::OutOfRange => ("decimal_parsing", "Input should be a valid decimal"),
        }
    }
}

pub(crate) fn integer(text: &str) -> IntegerText {
    let trimmed = text.trim();
    let (negative, unsigned) = strip_sign(trimmed);
    let (whole, fraction) = match unsigned.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (unsigned, None),
    };
    if let Some(fraction_digits) = fraction
        && (fraction_digits.is_empty() || !fraction_digits.bytes().all(|byte| byte == b'0'))
    {
        return IntegerText::NotAnInteger;
    }
    let Some((magnitude, overflow)) = accumulate(whole) else {
        return IntegerText::NotAnInteger;
    };
    if overflow {
        return if negative {
            IntegerText::Value(i64::MIN)
        } else {
            IntegerText::AboveSignedRange
        };
    }
    IntegerText::Value(if negative {
        0_i64.saturating_sub(magnitude)
    } else {
        magnitude
    })
}

fn strip_sign(text: &str) -> (bool, &str) {
    if let Some(rest) = text.strip_prefix('-') {
        return (true, rest);
    }
    (false, text.strip_prefix('+').unwrap_or(text))
}

fn accumulate(digits: &str) -> Option<(i64, bool)> {
    let mut magnitude: i64 = 0;
    let mut overflow = false;
    let mut after_digit = false;
    for byte in digits.bytes() {
        if byte.is_ascii_digit() {
            let digit = i64::from(byte.wrapping_sub(b'0'));
            match magnitude
                .checked_mul(10)
                .and_then(|value| value.checked_add(digit))
            {
                Some(value) => magnitude = value,
                None => overflow = true,
            }
            after_digit = true;
        } else if byte == b'_' && after_digit {
            after_digit = false;
        } else {
            return None;
        }
    }
    after_digit.then_some((magnitude, overflow))
}

pub(crate) fn money(text: &str) -> Result<Decimal, NumericIssue> {
    let normalized = normalized_number(text)?;
    let value: f64 = normalized
        .parse()
        .map_err(|_error| NumericIssue::NotANumber)?;
    if !value.is_finite() {
        return Err(NumericIssue::NotFinite);
    }
    if value.abs().to_bits() == 0 {
        return Ok(Decimal::ZERO);
    }
    Decimal::from_str(&normalized).map_err(|_error| NumericIssue::OutOfRange)
}

fn normalized_number(text: &str) -> Result<String, NumericIssue> {
    let trimmed = text.trim();
    let mut normalized = String::with_capacity(trimmed.len());
    let mut previous: Option<char> = None;
    let mut characters = trimmed.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '_' {
            let between_digits = previous.is_some_and(|earlier| earlier.is_ascii_digit())
                && characters.peek().is_some_and(char::is_ascii_digit);
            if !between_digits {
                return Err(NumericIssue::NotANumber);
            }
            continue;
        }
        normalized.push(character);
        previous = Some(character);
    }
    Ok(normalized)
}
