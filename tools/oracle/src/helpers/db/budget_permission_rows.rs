use betterlite_db::budget_permission_rows::{
    LiteLLMBudgetTable, LiteLLMBudgetTableFull, LiteLLMObjectPermissionTable,
    OrganizationMembership,
};
use rust_decimal::prelude::ToPrimitive as _;

pub(crate) struct Op;

#[derive(serde::Deserialize)]
#[serde(tag = "operation")]
pub(crate) enum Input {
    #[serde(rename = "budget")]
    Budget { row: LiteLLMBudgetTable },
    #[serde(rename = "full")]
    Full { row: LiteLLMBudgetTableFull },
    #[serde(rename = "permission")]
    Permission { row: LiteLLMObjectPermissionTable },
    #[serde(rename = "membership")]
    Membership { row: OrganizationMembership },
    #[serde(rename = "effective")]
    Effective {
        row: LiteLLMBudgetTable,
        now: chrono::DateTime<chrono::Utc>,
    },
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::Budget { row } => dump(row),
            Input::Full { row } => dump(row),
            Input::Permission { row } => dump(row),
            Input::Membership { row } => dump(row),
            Input::Effective { row, now } => {
                let active = row
                    .active_temp_budget_increase(*now)
                    .to_f64()
                    .ok_or_else(|| helper_error("InvalidOutput", "decimal conversion failed"))?;
                let effective = match row
                    .effective_max_budget(*now)
                    .map_err(|error| helper_error("InvalidOutput", &error.to_string()))?
                {
                    Some(value) => Some(value.to_f64().ok_or_else(|| {
                        helper_error("InvalidOutput", "decimal conversion failed")
                    })?),
                    None => None,
                };
                Ok(
                    serde_json::json!({"active_temp_budget_increase": active, "effective_max_budget": effective}),
                )
            }
        }
    }
}

fn dump<T: serde::Serialize>(value: &T) -> crate::helpers::HelperResult {
    let output = serde_json::to_value(value)
        .map_err(|error| helper_error("InvalidOutput", &error.to_string()))?;
    Ok(output)
}

fn helper_error(class: &str, message: &str) -> Box<crate::helpers::HelperError> {
    let error = crate::helpers::HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    };
    Box::new(error)
}
