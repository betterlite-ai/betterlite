use betterlite_db::spend_queues::entity::SpendAmount;
use chrono::NaiveDate;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

const START_TIME_KEY: &str = "startTime";
const SPEND_KEY: &str = "spend";
const USERS_KEY: &str = "users";
const MODELS_KEY: &str = "models";
// FastAPI writes a None dict key as the JSON key "null"
const NULL_USER_KEY: &str = "null";

// the optional filters _spend_logs_daily_summary_sql binds after the two time bounds; request_id matches only itself
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SummaryFilters<'filter> {
    pub api_key: Option<&'filter str>,
    pub request_id: Option<&'filter str>,
    pub user_id: Option<&'filter str>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SummaryQuery {
    pub sql: String,
    pub params: Vec<String>,
}

// one row of the grouped query, in its column names
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DailySummaryRow {
    pub day: String,
    pub api_key: Option<String>,
    pub user: Option<String>,
    pub model: String,
    pub spend: SpendAmount,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum SummaryValue {
    Spend(SpendAmount),
    Day(NaiveDate),
    Breakdown(IndexMap<String, SpendAmount>),
}

#[derive(Debug, thiserror::Error)]
pub enum SummaryError {
    #[error("the summary day {day:?} is not an ISO date")]
    Day {
        day: String,
        #[source]
        source: chrono::ParseError,
    },
    #[error("summing the daily spend overflowed Decimal's range")]
    Overflow,
}

// _spend_logs_daily_summary_sql: each present filter takes the next placeholder from $3 on
#[must_use]
pub fn daily_summary_sql(
    start_date_iso: &str,
    end_date_iso: &str,
    filters: SummaryFilters<'_>,
) -> SummaryQuery {
    let present = [
        ("api_key", filters.api_key),
        ("request_id", filters.request_id),
        ("\"user\"", filters.user_id),
    ];
    let mut params = vec![start_date_iso.to_owned(), end_date_iso.to_owned()];
    let mut clauses = Vec::new();
    for (column, value) in present {
        let Some(bound) = value else {
            continue;
        };
        params.push(bound.to_owned());
        clauses.push(format!("AND {column} = ${}", params.len()));
    }
    let filter_sql = clauses.join("\n");
    let sql = format!(
        "\nSELECT\n    to_char(date_trunc('day', \"startTime\"), 'YYYY-MM-DD') AS day,\n    api_key,\n    \"user\",\n    model,\n    SUM(spend) AS spend\nFROM \"LiteLLM_SpendLogs\"\nWHERE \"startTime\" >= ($1::timestamptz AT TIME ZONE 'UTC') AND \"startTime\" <= ($2::timestamptz AT TIME ZONE 'UTC')\n{filter_sql}\nGROUP BY 1, 2, 3, 4\nORDER BY 1\n"
    );
    SummaryQuery { sql, params }
}

// itertools.groupby over the day column: only neighbouring rows of one day share an item
pub fn daily_summary_items(
    rows: &[DailySummaryRow],
) -> Result<Vec<IndexMap<String, SummaryValue>>, SummaryError> {
    let mut items = Vec::new();
    for day_rows in rows.chunk_by(|earlier, later| earlier.day == later.day) {
        let Some(first) = day_rows.first() else {
            continue;
        };
        let day = NaiveDate::parse_from_str(&first.day, "%Y-%m-%d").map_err(|source| {
            SummaryError::Day {
                day: first.day.clone(),
                source,
            }
        })?;
        items.push(daily_summary_item(day, day_rows)?);
    }
    Ok(items)
}

// _daily_summary_item: API-key totals first, so a key named like a reserved field is overwritten in place
pub fn daily_summary_item(
    day: NaiveDate,
    rows: &[DailySummaryRow],
) -> Result<IndexMap<String, SummaryValue>, SummaryError> {
    let mut item = IndexMap::new();
    let keyed = sum_by(rows, |row| row.api_key.as_deref())?;
    for (api_key, spend) in keyed {
        item.insert(api_key, SummaryValue::Spend(spend));
    }
    let mut total = Decimal::ZERO;
    for row in rows {
        total = total
            .checked_add(row.spend.0)
            .ok_or(SummaryError::Overflow)?;
    }
    item.insert(START_TIME_KEY.to_owned(), SummaryValue::Day(day));
    item.insert(
        SPEND_KEY.to_owned(),
        SummaryValue::Spend(SpendAmount(total)),
    );
    let users = sum_by(rows, |row| {
        Some(row.user.as_deref().unwrap_or(NULL_USER_KEY))
    })?;
    item.insert(USERS_KEY.to_owned(), SummaryValue::Breakdown(users));
    let models = sum_by(rows, |row| Some(row.model.as_str()))?;
    item.insert(MODELS_KEY.to_owned(), SummaryValue::Breakdown(models));
    Ok(item)
}

// _sum_spend_by: a row the column leaves without a key is not summed
fn sum_by<'row, Column>(
    rows: &'row [DailySummaryRow],
    column: Column,
) -> Result<IndexMap<String, SpendAmount>, SummaryError>
where
    Column: Fn(&'row DailySummaryRow) -> Option<&'row str>,
{
    let mut sums: IndexMap<String, SpendAmount> = IndexMap::new();
    for row in rows {
        let Some(key) = column(row) else {
            continue;
        };
        let entry = sums
            .entry(key.to_owned())
            .or_insert(SpendAmount(Decimal::ZERO));
        entry.0 = entry
            .0
            .checked_add(row.spend.0)
            .ok_or(SummaryError::Overflow)?;
    }
    Ok(sums)
}
