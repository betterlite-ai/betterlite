use betterlite_db::daily_activity_contracts::{DailyActivityScope, SqlParameter, SqlQuery};
use betterlite_db::daily_activity_sql::DailyActivitySqlError;
use betterlite_db::daily_activity_sql::filters::{adjust_dates_for_timezone, build_where_clause};
use betterlite_db::daily_activity_sql::metrics::{
    KEY_SPEND_SELECT, ptu_flat_cost_select, rollup_metric_select,
};
use betterlite_db::daily_activity_sql::key_page::build_key_page_sql;
use betterlite_db::daily_activity_sql::rollup::{Rollup, build_rollup_sql};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub(crate) struct Op;
#[derive(Deserialize)]
pub(crate) struct Input {
    cases: BTreeMap<String, Case>,
}
#[derive(Deserialize)]
struct Case {
    scope: DailyActivityScope,
    utc_now: DateTime<Utc>,
    #[serde(flatten)]
    action: Action,
}
#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum Action {
    Dates,
    Where { start_index: i64 },
    Aggregate { limit: i64 },
    Entity { limit: i64 },
    Page { limit: i64, offset: i64 },
    Ptu { aggregate: bool },
    Metrics,
    KeyMetrics,
}
#[derive(Serialize)]
struct Outcome {
    query: Option<SqlQuery>,
    error: Option<String>,
}
impl crate::helpers::Helper for Op {
    type Input = Input;
    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let outcomes: BTreeMap<_, _> = input
            .cases
            .iter()
            .map(|(name, case)| {
                let outcome = match evaluate(case) {
                    Ok(mut query) => {
                        query.sql = query.sql.split_whitespace().collect::<Vec<_>>().join(" ");
                        Outcome {
                            query: Some(query),
                            error: None,
                        }
                    }
                    Err(error) => Outcome {
                        query: None,
                        error: Some(error.to_string()),
                    },
                };
                (name, outcome)
            })
            .collect();
        serde_json::to_value(outcomes).map_err(|error| {
            Box::new(crate::helpers::HelperError {
                class: "ValueError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
    }
}
fn evaluate(case: &Case) -> Result<SqlQuery, DailyActivitySqlError> {
    let scope = &case.scope;
    let input = scope.as_ref();
    let clock = || case.utc_now;
    let sql = match case.action {
        Action::Dates => {
            let (start, end) = adjust_dates_for_timezone(
                &input.start_date,
                &input.end_date,
                input.timezone_offset_minutes,
                input.include_current_utc_day,
                &clock,
            )?;
            return Ok(SqlQuery {
                sql: String::new(),
                params: vec![SqlParameter::Text(start), SqlParameter::Text(end)],
            });
        }
        Action::Where { start_index } => return build_where_clause(scope, start_index, &clock),
        Action::Aggregate { limit } => {
            return build_rollup_sql(scope, limit, Rollup::Aggregate, &clock);
        }
        Action::Entity { limit } => return build_rollup_sql(scope, limit, Rollup::Entity, &clock),
        Action::Page { limit, offset } => return build_key_page_sql(scope, offset, limit, &clock),
        Action::Ptu { aggregate } => ptu_flat_cost_select(input.table, aggregate),
        Action::Metrics => rollup_metric_select(input.table),
        Action::KeyMetrics => KEY_SPEND_SELECT.to_owned(),
    };
    Ok(SqlQuery {
        sql,
        params: Vec::new(),
    })
}
