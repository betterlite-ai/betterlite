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

fn plain(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

impl From<serde_json::Error> for Box<HelperError> {
    fn from(error: serde_json::Error) -> Self {
        plain("ValidationError", error.to_string())
    }
}

impl From<io::Error> for Box<HelperError> {
    fn from(error: io::Error) -> Self {
        plain("OSError", error.to_string())
    }
}
