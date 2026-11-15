use betterlite_db::daily_activity_contracts::identifiers::{
    DailyActivityError, DailyActivityTable,
};
use betterlite_db::daily_activity_contracts::{DailyActivityScope, DailyActivityScopeInput};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeIds {
    One(String),
    Many(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeRequest<'text> {
    pub table: DailyActivityTable,
    pub entity_id_field: &'text str,
    pub entity_id: Option<ScopeIds>,
    pub exclude_entity_ids: Vec<String>,
    pub api_key: Option<ScopeIds>,
    pub start_date: &'text str,
    pub end_date: &'text str,
    pub model: Option<&'text str>,
    pub timezone_offset_minutes: Option<i64>,
    pub include_current_utc_day: bool,
}

impl ScopeIds {
    fn into_values(self) -> Vec<String> {
        match self {
            Self::One(value) => vec![value],
            Self::Many(values) => values,
        }
    }
}

pub fn daily_activity_scope(
    request: ScopeRequest<'_>,
) -> Result<DailyActivityScope, DailyActivityError> {
    let entity_ids = request.entity_id.map(ScopeIds::into_values);
    let api_keys = match request.api_key {
        None => None,
        Some(ScopeIds::One(key)) if key.is_empty() => None,
        Some(ids) => Some(ids.into_values()),
    };
    DailyActivityScope::try_from(DailyActivityScopeInput {
        table: request.table,
        entity_id_field: request.entity_id_field.to_owned(),
        entity_ids,
        api_keys,
        exclude_entity_ids: request.exclude_entity_ids,
        start_date: request.start_date.to_owned(),
        end_date: request.end_date.to_owned(),
        model: request.model.map(str::to_owned),
        timezone_offset_minutes: request.timezone_offset_minutes,
        include_current_utc_day: request.include_current_utc_day,
    })
}
