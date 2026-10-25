use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use std::collections::HashMap;
use std::hash::BuildHasher;
use std::str;
use std::str::FromStr as _;
use std::time::SystemTime;
mod tests;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CounterEntityType {
    Key,
    Team,
    TeamMember,
    User,
    EndUser,
    Tag,
    ModelAccessGroup,
    Organization,
    Project,
}
impl CounterEntityType {
    #[must_use]
    pub fn from_litellm_name(name: &str) -> Option<Self> {
        match name {
            "Key" => Some(Self::Key),
            "Team" => Some(Self::Team),
            "TeamMember" => Some(Self::TeamMember),
            "User" => Some(Self::User),
            "EndUser" => Some(Self::EndUser),
            "Tag" => Some(Self::Tag),
            "Model access group" => Some(Self::ModelAccessGroup),
            "Organization" => Some(Self::Organization),
            "Project" => Some(Self::Project),
            _ => None,
        }
    }
    #[must_use]
    pub const fn wire_value(self) -> &'static str {
        match self {
            Self::Key => "key",
            Self::Team => "team",
            Self::TeamMember => "team_member",
            Self::User => "user",
            Self::EndUser => "end_user",
            Self::Tag => "tag",
            Self::ModelAccessGroup => "model_access_group",
            Self::Organization => "organization",
            Self::Project => "project",
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BudgetCounter {
    pub counter_key: String,
    pub max_budget: Decimal,
    pub fallback_spend: Decimal,
    pub entity_type: CounterEntityType,
    pub entity_id: String,
    pub source_cache_key: Option<String>,
    pub spend_log_entity_id: Option<String>,
    pub window_duration: Option<String>,
    pub window_start: Option<SystemTime>,
}
pub trait FloatInput {
    fn decimal_value(self) -> Option<Decimal>;
}
macro_rules! float_special { ($($type:ty => $value:expr),*) => { $(impl FloatInput for $type { fn decimal_value(self) -> Option<Decimal> { let value = self; $value(value) } })* }; }
float_special!(f64 => |value| Decimal::try_from(value).ok(), f32 => |value| Decimal::try_from(value).ok(), bool => |value| Some(if value { Decimal::ONE } else { Decimal::ZERO }));
macro_rules! float_integer {
    ($($type:ty),*) => {
        $(impl FloatInput for $type {
            fn decimal_value(self) -> Option<Decimal> {
                Some(Decimal::from(self))
            }
        })*
    };
}
float_integer!(
    i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize
);
fn parse_decimal_text(text: &str) -> Option<Decimal> {
    let trimmed = text.trim_ascii();
    let without_digit_grouping = trimmed.replace('_', "");
    Decimal::from_str(&without_digit_grouping).ok()
}
macro_rules! float_string { ($($type:ty),*) => { $(impl FloatInput for $type { fn decimal_value(self) -> Option<Decimal> { parse_decimal_text(self.as_ref()) } })* }; }
float_string!(String, &str);
macro_rules! float_bytes { ($($type:ty),*) => { $(impl FloatInput for $type { fn decimal_value(self) -> Option<Decimal> { parse_decimal_text(str::from_utf8(self.as_ref()).ok()?) } })* }; }
float_bytes!(Vec<u8>, &[u8]);
pub fn _to_float<T>(value: T) -> Option<Decimal>
where
    T: FloatInput,
{
    let decimal = value.decimal_value()?;
    Some(decimal)
}
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
macro_rules! int_string { ($($type:ty),*) => { $(impl IntInput for $type { fn int_value(self) -> Option<i64> { self.trim_ascii().replace('_', "").parse().ok() } })* }; }
int_string!(String, &str);
macro_rules! int_bytes { ($($type:ty),*) => { $(impl IntInput for $type { fn int_value(self) -> Option<i64> { str::from_utf8(self.as_ref()).ok()?.trim_ascii().replace('_', "").parse().ok() } })* }; }
int_bytes!(Vec<u8>, &[u8]);
pub fn _to_int<T>(value: T) -> Option<i64>
where
    T: IntInput,
{
    if let Some(integer) = value.int_value() {
        return Some(integer);
    }
    None
}
pub trait ValueLookup {
    fn value(&self, key: &str) -> Option<&String>;
}
impl<S: BuildHasher> ValueLookup for HashMap<String, String, S> {
    fn value(&self, key: &str) -> Option<&String> {
        self.get(key)
    }
}
pub fn _get_value<'value, T>(object: &'value T, key: &str) -> Option<&'value String>
where
    T: ValueLookup + ?Sized,
{
    let lookup_key = key.to_owned();
    object.value(&lookup_key)
}
