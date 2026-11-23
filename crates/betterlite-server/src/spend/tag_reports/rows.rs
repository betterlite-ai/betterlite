use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::table_ops::rows::UserWithAliases;
use rust_decimal::Decimal;
use serde::Serialize;

// get_spend_by_tags: one expanded request tag with its occurrence count and summed spend
#[derive(Clone, Debug, Serialize)]
pub struct RawTagSpend {
    pub individual_request_tag: Option<String>,
    pub log_count: i64,
    #[serde(serialize_with = "rust_decimal::serde::arbitrary_precision::serialize")]
    pub total_spend: Decimal,
}

// one DailyTagSpend row, or one tag after the filtered query's SUM and GROUP BY
#[derive(Clone, Debug)]
pub struct DailyTagSpend {
    pub individual_request_tag: Option<String>,
    pub total_spend: Decimal,
    pub log_count: i64,
}

// get_data(table_name="user", query_type="find_all"): u.* flattened next to key_aliases
#[derive(Clone, Debug, Serialize)]
pub struct KeyAliasedUser {
    #[serde(flatten)]
    pub user: LiteLLMUserTable,
    pub key_aliases: Vec<Option<String>>,
}

impl From<UserWithAliases> for KeyAliasedUser {
    fn from(stored: UserWithAliases) -> Self {
        Self {
            user: stored.user,
            key_aliases: stored.key_aliases,
        }
    }
}

#[derive(Serialize)]
pub struct TagNamesBody {
    pub tag_names: Vec<Option<String>>,
}

#[derive(Serialize)]
pub struct TagSpendBody {
    pub spend_per_tag: Vec<TagSpendRow>,
}

#[derive(Serialize)]
pub struct TagSpendRow {
    pub name: Option<String>,
    #[serde(serialize_with = "rust_decimal::serde::arbitrary_precision::serialize")]
    pub spend: Decimal,
    pub log_count: i64,
}
