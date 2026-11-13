use std::fmt::Display;
use std::io;

type OptionalString = Option<Option<String>>;
type OptionalCode = Option<Option<ErrorCode>>;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct HelperError {
    pub class: String,
    pub status: Option<u16>,
    #[serde(
        rename = "type",
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub error_type: OptionalString,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub param: OptionalString,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub code: OptionalCode,
    pub message: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum ErrorCode {
    Text(String),
    Number(serde_json::Number),
}

pub trait OrHelperError<T> {
    fn or_config_error(self) -> Result<T, Box<HelperError>>;
    fn or_value_error(self) -> Result<T, Box<HelperError>>;
}

enum ErrorClass {
    Config,
    Value,
}

fn map_helper_error<T, E>(result: Result<T, E>, class: &ErrorClass) -> Result<T, Box<HelperError>>
where
    E: Display,
{
    result.map_err(|error| {
        let message = error.to_string();
        Box::new(match class {
            ErrorClass::Config => HelperError {
                class: "ConfigError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message,
            },
            ErrorClass::Value => HelperError {
                class: "ValueError".to_owned(),
                status: None,
                error_type: Some(None),
                param: Some(None),
                code: Some(None),
                message,
            },
        })
    })
}

impl<T, E: Display> OrHelperError<T> for Result<T, E> {
    fn or_config_error(self) -> Result<T, Box<HelperError>> {
        map_helper_error(self, &ErrorClass::Config)
    }

    fn or_value_error(self) -> Result<T, Box<HelperError>> {
        map_helper_error(self, &ErrorClass::Value)
    }
}

macro_rules! from_error {
    ($error:ty, $class:literal) => {
        impl From<$error> for Box<HelperError> {
            fn from(error: $error) -> Self {
                Self::new(HelperError {
                    class: $class.to_owned(),
                    status: None,
                    error_type: None,
                    param: None,
                    code: None,
                    message: error.to_string(),
                })
            }
        }
    };
}

from_error!(serde_json::Error, "ValidationError");
from_error!(io::Error, "OSError");

mod tests;
