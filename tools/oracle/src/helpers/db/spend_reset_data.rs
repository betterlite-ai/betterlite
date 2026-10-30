use betterlite_db::spend_reset_unit_of_work::spend_reset_data;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::Deserialize;
use std::collections::BTreeMap;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    cases: BTreeMap<String, Case>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    budget_reset_at: Option<DateTime<Utc>>,
    spend_decrement: Decimal,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let mut output = BTreeMap::new();
        for (name, case) in &input.cases {
            output.insert(
                name,
                spend_reset_data(case.budget_reset_at, case.spend_decrement),
            );
        }
        serde_json::to_value(output).map_err(|error| {
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
