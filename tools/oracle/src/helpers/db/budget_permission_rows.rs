use betterlite_db::budget_permission_rows::{
    JsonData, LiteLLMBudgetTable, LiteLLMBudgetTableFull, LiteLLMObjectPermissionTable,
    OrganizationMembership,
};
use rust_decimal::prelude::ToPrimitive as _;

pub(crate) struct Op;

#[derive(serde::Deserialize)]
#[serde(tag = "operation")]
pub(crate) enum Input {
    #[serde(rename = "budget")]
    Budget { row: Validated<LiteLLMBudgetTable> },
    #[serde(rename = "full")]
    Full {
        row: Validated<LiteLLMBudgetTableFull>,
    },
    #[serde(rename = "permission")]
    Permission {
        row: Validated<LiteLLMObjectPermissionTable>,
    },
    #[serde(rename = "membership")]
    Membership {
        row: Validated<OrganizationMembership>,
    },
    #[serde(rename = "effective")]
    Effective {
        row: Validated<LiteLLMBudgetTable>,
        now: chrono::DateTime<chrono::Utc>,
    },
}

#[derive(serde::Deserialize)]
#[serde(untagged)]
pub(crate) enum Validated<T> {
    Valid(T),
    Invalid(JsonData),
}

impl<T> Validated<T> {
    fn get(&self) -> Result<&T, Box<crate::helpers::HelperError>> {
        match self {
            Self::Valid(value) => Ok(value),
            Self::Invalid(value) => Err(helper_error(
                "ValidationError",
                &format!("invalid row: {value:?}"),
            )),
        }
    }
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::Budget { row } => serde_json::to_value(row.get()?)
                .map_err(|error| helper_error("InvalidOutput", &error.to_string())),
            Input::Full { row } => serde_json::to_value(row.get()?)
                .map_err(|error| helper_error("InvalidOutput", &error.to_string())),
            Input::Permission { row } => serde_json::to_value(row.get()?)
                .map_err(|error| helper_error("InvalidOutput", &error.to_string())),
            Input::Membership { row } => serde_json::to_value(row.get()?)
                .map_err(|error| helper_error("InvalidOutput", &error.to_string())),
            Input::Effective { row, now } => {
                let budget = row.get()?;
                let active = budget
                    .active_temp_budget_increase(*now)
                    .to_f64()
                    .ok_or_else(|| helper_error("InvalidOutput", "decimal conversion failed"))?;
                let effective = match budget
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
