use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DiagnosticError {
    #[serde(rename = "type")]
    pub error_type: String,
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct UnsupportedSnapshot {
    pub error: DiagnosticError,
}
