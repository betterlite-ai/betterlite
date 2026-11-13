use std::collections::BTreeMap;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::budget_permission_rows::{JsonData, LiteLLMBudgetTable};

use super::ManagedRowsDecodeError;
use super::wire::RowTimestamp;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagTable {
    pub tag_name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub models: Vec<String>,
    #[serde(default)]
    pub model_info: Option<BTreeMap<String, JsonData>>,
    #[serde(with = "rust_decimal::serde::float")]
    pub spend: Decimal,
    #[serde(default)]
    pub budget_id: Option<String>,
    #[serde(default)]
    pub litellm_budget_table: Option<LiteLLMBudgetTable>,
    #[serde(default)]
    pub created_at: Option<RowTimestamp>,
    #[serde(default)]
    pub created_by: Option<String>,
    #[serde(default)]
    pub updated_at: Option<RowTimestamp>,
}

pub(crate) fn decode_tag(row: &mut JsonData) -> Result<TagTable, ManagedRowsDecodeError> {
    let fields = super::object_fields(row)?;
    replace_when_absent_or_null(fields, "models", JsonData::Array(Vec::new()));
    replace_when_absent_or_null(
        fields,
        "spend",
        JsonData::Number(serde_json::Number::from(0_i32)),
    );
    super::decode_row(row)
}

fn replace_when_absent_or_null(
    fields: &mut BTreeMap<String, JsonData>,
    field: &'static str,
    replacement: JsonData,
) {
    if fields
        .get(field)
        .is_none_or(|value| matches!(value, JsonData::Null))
    {
        let _previous = fields.insert(field.to_owned(), replacement);
    }
}
