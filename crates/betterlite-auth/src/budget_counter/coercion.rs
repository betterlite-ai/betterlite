use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;
use rust_decimal::prelude::ToPrimitive as _;
use std::collections::HashMap;
use std::hash::BuildHasher;
use std::str;
use std::str::FromStr as _;

pub trait FloatInput {
    fn decimal_value(self) -> Option<Decimal>;
}

macro_rules! float_decimal {
    ($($type:ty),*) => {
        $(impl FloatInput for $type {
            fn decimal_value(self) -> Option<Decimal> {
                Decimal::try_from(self).ok()
            }
        })*
    };
}
float_decimal!(f64, f32);

impl FloatInput for bool {
    fn decimal_value(self) -> Option<Decimal> {
        Some(if self { Decimal::ONE } else { Decimal::ZERO })
    }
}

macro_rules! float_integer {
    ($($type:ty),*) => {
        $(impl FloatInput for $type {
            fn decimal_value(self) -> Option<Decimal> {
                Some(Decimal::from(self))
            }
        })*
    };
}
float_integer!(i8, i16, i32, i64, isize, u8, u16, u32, u64, usize);

macro_rules! float_128 {
    ($($type:ty => $method:ident),*) => {
        $(impl FloatInput for $type {
            fn decimal_value(self) -> Option<Decimal> {
                Decimal::$method(self)
            }
        })*
    };
}
float_128!(i128 => from_i128, u128 => from_u128);

fn parse_decimal_text(text: &str) -> Option<Decimal> {
    let normalized = normalize_digit_grouping(text.trim_ascii())?;
    Decimal::from_str(&normalized)
        .or_else(|_| Decimal::from_scientific(&normalized))
        .ok()
}

macro_rules! float_string {
    ($($type:ty),*) => {
        $(impl FloatInput for $type {
            fn decimal_value(self) -> Option<Decimal> {
                parse_decimal_text(self.as_ref())
            }
        })*
    };
}
float_string!(String, &str);

macro_rules! float_bytes {
    ($($type:ty),*) => {
        $(impl FloatInput for $type {
            fn decimal_value(self) -> Option<Decimal> {
                parse_decimal_text(str::from_utf8(self.as_ref()).ok()?)
            }
        })*
    };
}
float_bytes!(Vec<u8>, &[u8]);

pub trait IntInput {
    fn int_value(self) -> Option<i64>;
}

macro_rules! int_numeric {
    ($($type:ty),*) => {
        $(impl IntInput for $type {
            fn int_value(self) -> Option<i64> {
                self.to_i64()
            }
        })*
    };
}
int_numeric!(
    i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize, f32, f64
);

impl IntInput for bool {
    fn int_value(self) -> Option<i64> {
        Some(i64::from(self))
    }
}

macro_rules! int_string {
    ($($type:ty),*) => {
        $(impl IntInput for $type {
            fn int_value(self) -> Option<i64> {
                normalize_digit_grouping(self.trim_ascii())?
                    .parse::<i64>()
                    .ok()
            }
        })*
    };
}
int_string!(String, &str);

macro_rules! int_bytes {
    ($($type:ty),*) => {
        $(impl IntInput for $type {
            fn int_value(self) -> Option<i64> {
                normalize_digit_grouping(str::from_utf8(self.as_ref()).ok()?.trim_ascii())?
                    .parse::<i64>()
                    .ok()
            }
        })*
    };
}
int_bytes!(Vec<u8>, &[u8]);

fn normalize_digit_grouping(text: &str) -> Option<String> {
    let mut parts = text.split('_');
    let mut previous = parts.next()?;
    let mut normalized = previous.to_owned();
    for part in parts {
        if !previous.ends_with(|character: char| character.is_ascii_digit())
            || !part.starts_with(|character: char| character.is_ascii_digit())
        {
            return None;
        }
        normalized.push_str(part);
        previous = part;
    }
    Some(normalized)
}

pub trait ValueLookup {
    fn value(&self, key: &str) -> Option<&String>;
}

impl<S: BuildHasher> ValueLookup for HashMap<String, String, S> {
    fn value(&self, key: &str) -> Option<&String> {
        self.get(key)
    }
}

mod tests;
