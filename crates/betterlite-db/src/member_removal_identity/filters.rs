use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FieldFilter {
    Eq(String),
    In {
        #[serde(rename = "in")]
        values: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct FilterClause(pub BTreeMap<String, FieldFilter>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnyFilter {
    #[serde(rename = "OR")]
    pub or: Vec<FilterClause>,
}

#[must_use]
pub fn eq_filter(field: &str, value: String) -> FilterClause {
    let mut fields = BTreeMap::new();
    fields.insert(field.to_owned(), FieldFilter::Eq(value));
    FilterClause(fields)
}

#[must_use]
pub fn in_filter(field: &str, values: Vec<String>) -> FilterClause {
    let mut sorted = values;
    sorted.sort();
    let mut fields = BTreeMap::new();
    fields.insert(field.to_owned(), FieldFilter::In { values: sorted });
    FilterClause(fields)
}

#[must_use]
pub fn team_users_filter(team_id: String, user_ids: Vec<String>) -> FilterClause {
    let mut clause = in_filter("user_id", user_ids);
    clause
        .0
        .insert("team_id".to_owned(), FieldFilter::Eq(team_id));
    clause
}
