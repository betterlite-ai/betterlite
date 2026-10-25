use betterlite_db::daily_activity_contracts::{DailyActivityScope, DailyActivityScopeInput};
use betterlite_db::daily_activity_sql::filters::build_where_clause;
use serde::Deserialize;

#[derive(Deserialize)]
pub(crate) struct Input {
    function: String,
    scope: DailyActivityScopeInput,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let scope = DailyActivityScope::try_from(input.scope.clone()).map_err(|error| {
            Box::new(crate::helpers::HelperError {
                class: "ValueError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })?;
        if input.function != "where" {
            return Err(Box::new(crate::helpers::HelperError {
                class: "ValueError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: "unknown function".to_owned(),
            }));
        }
        let (sql, params) = build_where_clause(&scope);
        serde_json::to_value(serde_json::json!({"sql": sql, "params": params})).map_err(|error| {
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
