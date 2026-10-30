use betterlite_db::identity_rows::{
    end_user::LiteLLMEndUserTable,
    key::{DeletedVerificationToken, LiteLLMVerificationToken},
    user::{LiteLLMUserTable, ScimPlaceholder},
};

pub(crate) struct Op;

#[derive(serde::Deserialize)]
pub(crate) struct Input {
    operation: String,
    row: serde_json::Value,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let output = match input.operation.as_str() {
            "key" => serde_json::to_value(decode::<LiteLLMVerificationToken>(&input.row)?)
                .map_err(|error| helper_error("InvalidOutput", &error.to_string())),
            "deleted_key" => serde_json::to_value(decode::<DeletedVerificationToken>(&input.row)?)
                .map_err(|error| helper_error("InvalidOutput", &error.to_string())),
            "user" => {
                let user = decode::<LiteLLMUserTable>(&input.row)?;
                let mut value = serde_json::to_value(user)
                    .map_err(|error| helper_error("InvalidOutput", &error.to_string()))?;
                if let serde_json::Value::Object(fields) = &mut value {
                    fields.insert(
                        "password_present".to_owned(),
                        serde_json::Value::Bool(input.row.get("password").is_some()),
                    );
                }
                Ok(value)
            }
            "placeholder" => serde_json::to_value(decode::<ScimPlaceholder>(&input.row)?)
                .map_err(|error| helper_error("InvalidOutput", &error.to_string())),
            "end_user" => serde_json::to_value(decode::<LiteLLMEndUserTable>(&input.row)?)
                .map_err(|error| helper_error("InvalidOutput", &error.to_string())),
            _ => {
                return Err(helper_error(
                    "InvalidInput",
                    "unsupported identity row operation",
                ));
            }
        };
        output
    }
}

fn decode<T: serde::de::DeserializeOwned>(
    value: &serde_json::Value,
) -> Result<T, Box<crate::helpers::HelperError>> {
    let decoded = serde_json::from_value(value.clone());
    decoded.map_err(|error| helper_error("ValidationError", &error.to_string()))
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
