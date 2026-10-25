use std::collections::HashMap;
use std::hash::BuildHasher;
use std::str;
use std::time::SystemTime;
mod tests;
#[derive(Clone, Debug, PartialEq)]
pub struct BudgetCounter {
    pub counter_key: String,
    pub max_budget: f64,
    pub fallback_spend: f64,
    pub entity_type: String,
    pub entity_id: String,
    pub source_cache_key: Option<String>,
    pub spend_log_entity_id: Option<String>,
    pub window_duration: Option<String>,
    pub window_start: Option<SystemTime>,
}
pub const _COUNTER_ENTITY_TYPES: [(&str, &str); 9] = [
    ("Key", "key"),
    ("Team", "team"),
    ("TeamMember", "team_member"),
    ("User", "user"),
    ("EndUser", "end_user"),
    ("Tag", "tag"),
    ("Model access group", "model_access_group"),
    ("Organization", "organization"),
    ("Project", "project"),
];
pub trait FloatInput {
    fn float_value(self) -> Option<f64>;
}
impl FloatInput for f64 {
    fn float_value(self) -> Option<f64> {
        Some(self)
    }
}
impl FloatInput for f32 {
    fn float_value(self) -> Option<f64> {
        Some(f64::from(self))
    }
}
impl FloatInput for bool {
    fn float_value(self) -> Option<f64> {
        Some(if self { 1.0 } else { 0.0 })
    }
}

macro_rules! float_integer {
    ($($type:ty),*) => {
        $(impl FloatInput for $type {
            fn float_value(self) -> Option<f64> {
                self.to_string().parse().ok()
            }
        })*
    };
}

float_integer!(
    i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize
);
macro_rules! float_text {
    ($($type:ty),*) => {
        $(impl FloatInput for $type {
            fn float_value(self) -> Option<f64> { self.parse().ok() }
        })*
    };
}

float_text!(String, &str);
impl FloatInput for Vec<u8> {
    fn float_value(self) -> Option<f64> {
        str::from_utf8(&self).ok()?.parse().ok()
    }
}

impl FloatInput for &[u8] {
    fn float_value(self) -> Option<f64> {
        str::from_utf8(self).ok()?.parse().ok()
    }
}
pub fn _to_float<T>(value: T) -> Option<f64>
where
    T: FloatInput,
{
    let result = value.float_value();
    result.or(None)
}

pub trait IntInput {
    fn int_value(self) -> Option<i64>;
}

macro_rules! int_integer {
    ($($type:ty),*) => {
        $(impl IntInput for $type {
            fn int_value(self) -> Option<i64> {
                i64::try_from(self).ok()
            }
        })*
    };
}

int_integer!(
    i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize
);

macro_rules! int_float {
    ($($type:ty),*) => {
        $(impl IntInput for $type {
            fn int_value(self) -> Option<i64> {
                self.trunc().to_string().parse().ok()
            }
        })*
    };
}

int_float!(f32, f64);

impl IntInput for bool {
    fn int_value(self) -> Option<i64> {
        Some(i64::from(self))
    }
}

macro_rules! int_text {
    ($($type:ty),*) => {
        $(impl IntInput for $type {
            fn int_value(self) -> Option<i64> { self.parse().ok() }
        })*
    };
}

int_text!(String, &str);

impl IntInput for Vec<u8> {
    fn int_value(self) -> Option<i64> {
        str::from_utf8(&self).ok()?.parse().ok()
    }
}

impl IntInput for &[u8] {
    fn int_value(self) -> Option<i64> {
        str::from_utf8(self).ok()?.parse().ok()
    }
}

pub fn _to_int<T>(value: T) -> Option<i64>
where
    T: IntInput,
{
    let result = value.int_value();
    result.or(None)
}

pub type BudgetValue = String;

pub trait ValueLookup {
    fn value(&self, key: &str) -> Option<&BudgetValue>;
}

impl<S: BuildHasher> ValueLookup for HashMap<String, BudgetValue, S> {
    fn value(&self, key: &str) -> Option<&BudgetValue> {
        self.get(key)
    }
}

pub fn _get_value<'value, T>(object: &'value T, key: &str) -> Option<&'value BudgetValue>
where
    T: ValueLookup + ?Sized,
{
    let result = object.value(key);
    result.or(None)
}
