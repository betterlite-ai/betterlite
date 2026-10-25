use betterlite_db::daily_activity_contracts::{self, DailyActivityRow, DailyActivityScope};
use serde::Deserialize;

pub(crate) struct Op;

#[derive(Deserialize)]
pub(crate) struct Input {
    step: Step,
}

#[derive(Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub(crate) enum Step {
    Scope(DailyActivityScope),
    KeySpend(daily_activity_contracts::KeySpendRow),
    KeyPage(daily_activity_contracts::KeyPage),
    KeyMetadata(daily_activity_contracts::KeyMetadataRow),
    Export(daily_activity_contracts::ExportRow),
    Rollup(daily_activity_contracts::RollupMetricsRow),
    Grouping(daily_activity_contracts::GroupingSetsRow),
    Entity(daily_activity_contracts::EntityRollupRow),
    Aggregated(daily_activity_contracts::AggregatedRows),
    ExportCursor(daily_activity_contracts::ExportCursor),
    SqlQuery(daily_activity_contracts::SqlQuery),
    DailyRow(DailyActivityRow),
    DailyPage(daily_activity_contracts::DailyRowsPage),
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let result = match &input.step {
            Step::Scope(value) => serde_json::to_value(value),
            Step::KeySpend(value) => serde_json::to_value(value),
            Step::KeyPage(value) => serde_json::to_value(value),
            Step::KeyMetadata(value) => serde_json::to_value(value),
            Step::Export(value) => serde_json::to_value(value),
            Step::Rollup(value) => serde_json::to_value(value),
            Step::Grouping(value) => serde_json::to_value(value),
            Step::Entity(value) => serde_json::to_value(value),
            Step::Aggregated(value) => serde_json::to_value(value),
            Step::ExportCursor(value) => serde_json::to_value(value),
            Step::SqlQuery(value) => serde_json::to_value(value),
            Step::DailyRow(value) => serde_json::to_value(value),
            Step::DailyPage(value) => serde_json::to_value(value),
        };
        result.map_err(|error| Self::error(error.to_string()))
    }
}

impl Op {
    fn error(message: String) -> Box<crate::helpers::HelperError> {
        let helper_error = crate::helpers::HelperError {
            class: "ValueError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message,
        };
        Box::new(helper_error)
    }
}
