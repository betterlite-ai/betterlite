use betterlite_db::daily_activity_contracts::{
    DailyActivityScope, ExportCursor, ExportType, SqlQuery,
};
use betterlite_db::daily_activity_export::{
    DailyActivityExportError, build_cache_leakage_keys_sql, build_export_sql, build_key_search_sql,
    build_model_top_keys_sql,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use chrono::{DateTime, Utc};

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
    Search {
        search: String,
        limit: i64,
    },
    Model {
        model_group: String,
        by_model_group: bool,
        limit: i64,
    },
    Leakage {
        limit: i64,
    },
    Export {
        export_type: ExportType,
        after: Option<ExportCursor>,
        batch_size: i64,
    },
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

fn evaluate(case: &Case) -> Result<SqlQuery, DailyActivityExportError> {
    let clock = || case.utc_now;
    match &case.action {
        Action::Search { search, limit } => build_key_search_sql(&case.scope, search, *limit, &clock),
        Action::Model {
            model_group,
            by_model_group,
            limit,
        } => build_model_top_keys_sql(&case.scope, model_group, *by_model_group, *limit, &clock),
        Action::Leakage { limit } => build_cache_leakage_keys_sql(&case.scope, *limit, &clock),
        Action::Export {
            export_type,
            after,
            batch_size,
        } => build_export_sql(&case.scope, *export_type, after.as_ref(), *batch_size, &clock),
    }
}
