use betterlite_db::identity_rows::{
    end_user::LiteLLMEndUserTable,
    key::{DeletedVerificationToken, LiteLLMVerificationToken},
    user::{LiteLLMUserTable, ScimPlaceholder},
};

use super::budget_permission_rows::Validated;

pub(crate) struct Op;

#[derive(serde::Deserialize)]
#[serde(tag = "operation")]
pub(crate) enum Input {
    #[serde(rename = "key")]
    Key {
        row: Validated<LiteLLMVerificationToken>,
    },
    #[serde(rename = "deleted_key")]
    DeletedKey {
        row: Validated<DeletedVerificationToken>,
    },
    #[serde(rename = "user")]
    User { row: Validated<LiteLLMUserTable> },
    #[serde(rename = "placeholder")]
    Placeholder { row: Validated<ScimPlaceholder> },
    #[serde(rename = "end_user")]
    EndUser { row: Validated<LiteLLMEndUserTable> },
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::Key { row } => serde_json::to_value(validated(row)?)
                .map_err(|error| helper_error("InvalidOutput", &error.to_string())),
            Input::DeletedKey { row } => serde_json::to_value(validated(row)?)
                .map_err(|error| helper_error("InvalidOutput", &error.to_string())),
            Input::User { row } => {
                let user = validated(row)?;
                let mut value = serde_json::to_value(user)
                    .map_err(|error| helper_error("InvalidOutput", &error.to_string()))?;
                if let serde_json::Value::Object(fields) = &mut value {
                    fields.insert(
                        "password_present".to_owned(),
                        serde_json::Value::Bool(user.password.is_some()),
                    );
                }
                Ok(value)
            }
            Input::Placeholder { row } => serde_json::to_value(validated(row)?)
                .map_err(|error| helper_error("InvalidOutput", &error.to_string())),
            Input::EndUser { row } => serde_json::to_value(validated(row)?)
                .map_err(|error| helper_error("InvalidOutput", &error.to_string())),
        }
    }
}

fn validated<T>(row: &Validated<T>) -> Result<&T, Box<crate::helpers::HelperError>> {
    match row {
        Validated::Valid(value) => Ok(value),
        Validated::Invalid(value) => Err(helper_error(
            "ValidationError",
            &format!("invalid row: {value:?}"),
        )),
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
