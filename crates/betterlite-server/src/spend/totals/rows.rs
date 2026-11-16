use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct DailySpend {
    pub date: NaiveDate,
    #[serde(serialize_with = "rust_decimal::serde::arbitrary_precision::serialize")]
    pub spend: Decimal,
}

#[derive(Clone, Serialize)]
pub struct KeyDailySpend {
    pub date: NaiveDate,
    #[serde(serialize_with = "rust_decimal::serde::arbitrary_precision::serialize")]
    pub spend: Decimal,
    pub api_key: String,
}

#[derive(Clone, Serialize)]
pub struct UserKeyDailySpend {
    pub date: NaiveDate,
    #[serde(serialize_with = "rust_decimal::serde::arbitrary_precision::serialize")]
    pub spend: Decimal,
    pub api_key: String,
    pub user: String,
}

impl From<(KeyDailySpend, String)> for UserKeyDailySpend {
    fn from((daily, user): (KeyDailySpend, String)) -> Self {
        Self {
            date: daily.date,
            spend: daily.spend,
            api_key: daily.api_key,
            user,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct TotalSpendRow {
    pub total_spend: Option<Decimal>,
}

#[derive(Clone, Serialize)]
pub struct KeySpend {
    pub api_key: String,
    #[serde(serialize_with = "rust_decimal::serde::arbitrary_precision::serialize")]
    pub total_spend: Decimal,
    pub key_alias: Option<String>,
    pub key_name: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ModelSpend {
    pub model: String,
    #[serde(serialize_with = "rust_decimal::serde::arbitrary_precision::serialize")]
    pub total_spend: Decimal,
}

#[derive(Clone, Debug, Serialize)]
pub struct UserModelSpend {
    pub model: String,
    #[serde(serialize_with = "rust_decimal::serde::arbitrary_precision::serialize")]
    pub total_spend: Decimal,
    pub total_tokens: i64,
}

#[derive(Clone, Debug)]
pub struct TeamSpend {
    pub team_alias: Option<String>,
    pub spend_date: Option<NaiveDate>,
    pub total_spend: Option<Decimal>,
}
