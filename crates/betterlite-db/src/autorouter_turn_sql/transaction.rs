use crate::daily_spend_sql::bind::SqlBind;
use chrono::{DateTime, FixedOffset};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

macro_rules! turn_record {
    ($($(#[$meta:meta])* $field:ident: $kind:ty => $bind:ident),+ $(,)?) => {
        #[derive(Clone, Serialize, Deserialize)]
        pub struct AutoRouterTurnTransaction { $($(#[$meta])* pub $field: $kind,)+ }
        pub const FIELD_ORDER: &[&str] = &[$(stringify!($field),)+];
        #[must_use]
        pub fn upsert_params(transaction: &AutoRouterTurnTransaction) -> Vec<SqlBind> {
            vec![$(turn_record!(@bind $bind, transaction.$field)),+]
        }
    };
    (@bind text, $value:expr) => { SqlBind::Text(Some($value.clone())) };
    (@bind optional, $value:expr) => { SqlBind::Text($value.clone()) };
    (@bind money, $value:expr) => { SqlBind::Money($value) };
    (@bind integer, $value:expr) => { SqlBind::Integer(Some($value)) };
    (@bind nullable, $value:expr) => { SqlBind::Integer($value) };
    (@bind flag, $value:expr) => { SqlBind::Integer(Some(i64::from($value.0))) };
    (@bind timestamp, $value:expr) => { SqlBind::Text(Some($value.to_rfc3339())) };
}
turn_record! {
    api_key: String => text,
    session_id: String => text,
    router_name: String => text,
    router_type: String => text,
    model: String => text,
    turn_at: DateTime<FixedOffset> => timestamp,
    total_tokens: i64 => integer,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    spend: Decimal => money,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    saved_spend: Decimal => money,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    classifier_cost: Decimal => money,
    covered: TurnFlag => flag,
    cache_hit: TurnFlag => flag,
    cache_ttl_seconds: Option<i64> => nullable,
    cache_touched: TurnFlag => flag,
    #[serde(default)]
    tier: Option<String> => optional,
    #[serde(default)]
    baseline_model: Option<String> => optional,
    #[serde(default)]
    savings_estimated_turns: i64 => integer,
    #[serde(default)]
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    savings_estimated_actual_spend: Decimal => money,
    #[serde(default)]
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    savings_estimated_saved_spend: Decimal => money,
    #[serde(default)]
    user_id: String => text,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TurnFlag(pub bool);
