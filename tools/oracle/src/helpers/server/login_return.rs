use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::login_return::origin::{ReturnToError, validate_return_to};
use betterlite_server::login_return::return_destination::is_same_origin_return_path;
use serde::Deserialize;
use serde_json::json;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    function: Function,
    return_to: String,
    #[serde(default)]
    control_plane_url: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    SameOriginPath,
    Validate,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input.function {
            Function::SameOriginPath => Ok(json!({
                "safe": is_same_origin_return_path(&input.return_to),
            })),
            Function::Validate => {
                let valid =
                    validate_return_to(&input.return_to, input.control_plane_url.as_deref())
                        .map_err(|error| return_to_failure(&error))?;
                Ok(json!({ "valid": valid }))
            }
        }
    }
}

fn return_to_failure(error: &ReturnToError) -> Box<HelperError> {
    let class = match error {
        ReturnToError::MismatchedOrigin => "HTTPException",
        ReturnToError::Parse(_) => "ValueError",
    };
    Box::new(HelperError {
        class: class.to_owned(),
        status: error.status(),
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
