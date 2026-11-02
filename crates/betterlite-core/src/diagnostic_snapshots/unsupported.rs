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

impl UnsupportedSnapshot {
    #[must_use]
    pub fn new(measurement: &str) -> Self {
        let error_type = "unsupported_runtime".to_owned();
        let code = "unsupported_runtime".to_owned();
        Self {
            error: DiagnosticError {
                error_type,
                code,
                message: format!("{measurement} requires CPython runtime introspection"),
            },
        }
    }
}
