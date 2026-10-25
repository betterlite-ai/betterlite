use crate::daily_activity_contracts::{DailyActivityScope, DailyActivityScopeInput, SqlParameter};
use chrono::{DateTime, Duration, Utc};

#[must_use]
pub fn adjust_dates_for_timezone(
    start_date: &str,
    end_date: &str,
    timezone_offset_minutes: Option<i64>,
    include_current_utc_day: bool,
    utc_now: Option<DateTime<Utc>>,
) -> (String, String) {
    if !include_current_utc_day || timezone_offset_minutes.is_none() {
        return (start_date.to_owned(), end_date.to_owned());
    }
    let now = utc_now.unwrap_or_else(Utc::now);
    let offset = timezone_offset_minutes.unwrap_or_default();
    let caller_today = now
        .checked_sub_signed(Duration::minutes(offset))
        .unwrap_or(now)
        .date_naive()
        .to_string();
    if end_date < caller_today.as_str() {
        return (start_date.to_owned(), end_date.to_owned());
    }
    let current_date = now.date_naive().to_string();
    let adjusted_end = if end_date > current_date.as_str() {
        end_date.to_owned()
    } else {
        current_date
    };
    (start_date.to_owned(), adjusted_end)
}

#[must_use]
pub fn build_where_clause(scope: &DailyActivityScope) -> (String, Vec<SqlParameter>) {
    build_where_clause_at(scope, 1, None)
}

#[must_use]
pub fn build_where_clause_at(
    scope: &DailyActivityScope,
    start_index: i64,
    utc_now: Option<DateTime<Utc>>,
) -> (String, Vec<SqlParameter>) {
    let input = scope.as_ref();
    let (start_date, end_date) = adjust_dates_for_timezone(
        &input.start_date,
        &input.end_date,
        input.timezone_offset_minutes,
        input.include_current_utc_day,
        utc_now,
    );
    let entity_index = start_index.saturating_add(2);
    let has_entities = input.entity_ids.as_ref().is_some_and(|ids| !ids.is_empty());
    let exclusion_index = entity_index.saturating_add(i64::from(has_entities));
    let has_exclusions = !input.exclude_entity_ids.is_empty();
    let model_index = exclusion_index.saturating_add(i64::from(has_exclusions));
    let has_model = input.model.as_ref().is_some_and(|model| !model.is_empty());
    let mut conditions = vec![
        format!("date >= ${start_index}"),
        format!("date <= ${}", start_index.saturating_add(1)),
    ];
    let mut params = vec![SqlParameter::Text(start_date), SqlParameter::Text(end_date)];
    append_entity_filter(input, entity_index, &mut conditions, &mut params);
    append_exclusion_filter(
        input,
        exclusion_index,
        has_exclusions,
        &mut conditions,
        &mut params,
    );
    append_model_filter(input, model_index, has_model, &mut conditions, &mut params);
    append_key_filter(input, model_index, has_model, &mut conditions, &mut params);
    (conditions.join(" AND "), params)
}

fn append_entity_filter(
    input: &DailyActivityScopeInput,
    entity_index: i64,
    conditions: &mut Vec<String>,
    params: &mut Vec<SqlParameter>,
) {
    match input.entity_ids.as_ref() {
        Some(ids) if ids.is_empty() => conditions.push("FALSE".to_owned()),
        Some(ids) => {
            conditions.push(format!(
                "\"{}\" = ANY(${entity_index}::text[])",
                input.entity_id_field
            ));
            params.push(SqlParameter::Texts(ids.clone()));
        }
        None => {}
    }
}

fn append_exclusion_filter(
    input: &DailyActivityScopeInput,
    exclusion_index: i64,
    has_exclusions: bool,
    conditions: &mut Vec<String>,
    params: &mut Vec<SqlParameter>,
) {
    if has_exclusions {
        conditions.push(format!(
            "(\"{}\" IS NULL OR NOT (\"{}\" = ANY(${exclusion_index}::text[])))",
            input.entity_id_field, input.entity_id_field
        ));
        params.push(SqlParameter::Texts(input.exclude_entity_ids.clone()));
    }
}

fn append_model_filter(
    input: &DailyActivityScopeInput,
    model_index: i64,
    has_model: bool,
    conditions: &mut Vec<String>,
    params: &mut Vec<SqlParameter>,
) {
    if has_model {
        conditions.push(format!("model = ${model_index}"));
        params.push(SqlParameter::Text(input.model.clone().unwrap_or_default()));
    }
}

fn append_key_filter(
    input: &DailyActivityScopeInput,
    model_index: i64,
    has_model: bool,
    conditions: &mut Vec<String>,
    params: &mut Vec<SqlParameter>,
) {
    let key_index = model_index.saturating_add(i64::from(has_model));
    match input.api_keys.as_ref() {
        Some(keys) if keys.is_empty() => conditions.push("FALSE".to_owned()),
        Some(keys) => {
            conditions.push(format!("api_key = ANY(${key_index}::text[])"));
            params.push(SqlParameter::Texts(keys.clone()));
        }
        None => {}
    }
}
