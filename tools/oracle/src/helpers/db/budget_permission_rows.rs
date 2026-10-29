use betterlite_db::budget_permission_rows::{LiteLLMBudgetTable, LiteLLMObjectPermissionTable};

pub(crate) struct Op;

#[derive(serde::Deserialize)]
pub(crate) struct Input {
    operation: String,
    row: serde_json::Value,
    #[serde(default)]
    now: Option<chrono::DateTime<chrono::Utc>>,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input.operation.as_str() {
            "budget" => dump(
                &serde_json::from_value::<LiteLLMBudgetTable>(input.row.clone())
                    .map_err(|error| helper_error("InvalidInput", &error.to_string()))?,
            ),
            "permission" => dump(
                &serde_json::from_value::<LiteLLMObjectPermissionTable>(input.row.clone())
                    .map_err(|error| helper_error("InvalidInput", &error.to_string()))?,
            ),
            "effective" => {
                let budget: LiteLLMBudgetTable = serde_json::from_value(input.row.clone())
                    .map_err(|error| helper_error("InvalidInput", &error.to_string()))?;
                let now = input
                    .now
                    .ok_or_else(|| helper_error("InvalidInput", "now is required"))?;
                Ok(
                    serde_json::json!({"active_temp_budget_increase": budget.active_temp_budget_increase(now), "effective_max_budget": budget.effective_max_budget(now)}),
                )
            }
            _ => Err(helper_error("InvalidInput", "unsupported operation")),
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
