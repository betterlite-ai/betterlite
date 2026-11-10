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
