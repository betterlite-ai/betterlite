use super::DailyActivitySqlError;
use crate::daily_activity_contracts::{DailyActivityScope, SqlParameter, SqlQuery};
use chrono::{DateTime, Datelike as _, Duration, Utc};

pub fn adjust_dates_for_timezone<Clock>(
    start_date: &str,
    end_date: &str,
    timezone_offset_minutes: Option<i64>,
    include_current_utc_day: bool,
    clock: &Clock,
) -> Result<(String, String), DailyActivitySqlError>
where
    Clock: Fn() -> DateTime<Utc>,
{
    let Some(offset) = timezone_offset_minutes.filter(|_| include_current_utc_day) else {
        return Ok((start_date.to_owned(), end_date.to_owned()));
    };
    let now = clock();
    let duration = Duration::try_minutes(offset).ok_or(DailyActivitySqlError::Timezone)?;
    let caller_today = now
        .checked_sub_signed(duration)
        .filter(|date| (1_i32..=9999).contains(&date.year()))
        .ok_or(DailyActivitySqlError::Timezone)?
        .date_naive()
        .to_string();
    let current_date = now.date_naive().to_string();
    let adjusted_end = if end_date >= caller_today.as_str() && end_date < current_date.as_str() {
        current_date
    } else {
        end_date.to_owned()
    };
    Ok((start_date.to_owned(), adjusted_end))
}

pub fn build_where_clause<Clock>(
    scope: &DailyActivityScope,
    start_index: i64,
    clock: &Clock,
) -> Result<SqlQuery, DailyActivitySqlError>
where
    Clock: Fn() -> DateTime<Utc>,
{
    let input = scope.as_ref();
    let (start, end) = adjust_dates_for_timezone(
        &input.start_date,
        &input.end_date,
        input.timezone_offset_minutes,
        input.include_current_utc_day,
        clock,
    )?;
    let index = i128::from(start_index);
    let mut filters = Conditions {
        index: index.saturating_add(2),
        clauses: vec![
            format!("date >= ${index}"),
            format!("date <= ${}", index.saturating_add(1)),
        ],
        params: vec![SqlParameter::Text(start), SqlParameter::Text(end)],
    };
    let entity = format!("\"{}\"", input.entity_id_field);
    filters.inclusion(&entity, input.entity_ids.as_deref());
    if !input.exclude_entity_ids.is_empty() {
        filters.clauses.push(format!(
            "({entity} IS NULL OR NOT ({entity} = ANY(${}::text[])))",
            filters.index
        ));
        filters
            .params
            .push(SqlParameter::Texts(input.exclude_entity_ids.clone()));
        filters.index = filters.index.saturating_add(1);
    }
    if let Some(model) = input.model.as_ref().filter(|model| !model.is_empty()) {
        filters.clauses.push(format!("model = ${}", filters.index));
        filters.params.push(SqlParameter::Text(model.clone()));
        filters.index = filters.index.saturating_add(1);
    }
    filters.inclusion("api_key", input.api_keys.as_deref());
    Ok(SqlQuery {
        sql: filters.clauses.join(" AND "),
        params: filters.params,
    })
}
struct Conditions {
    index: i128,
    clauses: Vec<String>,
    params: Vec<SqlParameter>,
}
impl Conditions {
    fn inclusion(&mut self, field: &str, values: Option<&[String]>) {
        match values {
            Some([]) => self.clauses.push("FALSE".to_owned()),
            Some(entries) => {
                self.clauses
                    .push(format!("{field} = ANY(${}::text[])", self.index));
                self.params.push(SqlParameter::Texts(entries.to_vec()));
                self.index = self.index.saturating_add(1);
            }
            None => {}
        }
    }
}
