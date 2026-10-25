use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;
use rust_decimal::prelude::ToPrimitive as _;
use std::collections::HashMap;
use std::hash::BuildHasher;
use std::str;
use std::str::FromStr as _;
use std::time::SystemTime;
mod tests;
#[derive(Clone, Debug, Eq, PartialEq)]
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
    pub const fn wire_value(&self) -> &'static str {
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
#[derive(Debug, Eq, PartialEq)]
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
float_integer!(i8, i16, i32, i64, isize, u8, u16, u32, u64, usize);
macro_rules! float_128 { ($($type:ty => $method:ident),*) => { $(impl FloatInput for $type { fn decimal_value(self) -> Option<Decimal> { Decimal::$method(self) } })* }; }
float_128!(i128 => from_i128, u128 => from_u128);
fn parse_decimal_text(text: &str) -> Option<Decimal> {
    let normalized = normalize_digit_grouping(text.trim_ascii())?;
    Decimal::from_str(&normalized)
        .or_else(|_| Decimal::from_scientific(&normalized))
        .ok()
}
macro_rules! float_string { ($($type:ty),*) => { $(impl FloatInput for $type { fn decimal_value(self) -> Option<Decimal> { parse_decimal_text(self.as_ref()) } })* }; }
float_string!(String, &str);
macro_rules! float_bytes { ($($type:ty),*) => { $(impl FloatInput for $type { fn decimal_value(self) -> Option<Decimal> { parse_decimal_text(str::from_utf8(self.as_ref()).ok()?) } })* }; }
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
macro_rules! int_string { ($($type:ty),*) => { $(impl IntInput for $type { fn int_value(self) -> Option<i64> { normalize_digit_grouping(self.trim_ascii())?.parse().ok() } })* }; }
int_string!(String, &str);
macro_rules! int_bytes { ($($type:ty),*) => { $(impl IntInput for $type { fn int_value(self) -> Option<i64> { normalize_digit_grouping(str::from_utf8(self.as_ref()).ok()?.trim_ascii())?.parse().ok() } })* }; }
int_bytes!(Vec<u8>, &[u8]);
fn normalize_digit_grouping(text: &str) -> Option<String> {
    let mut parts = text.split('_');
    let mut previous = parts.next()?;
    let mut normalized = previous.to_owned();
    for part in parts {
        if !previous.ends_with(|byte: char| byte.is_ascii_digit())
            || !part.starts_with(|byte: char| byte.is_ascii_digit())
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
