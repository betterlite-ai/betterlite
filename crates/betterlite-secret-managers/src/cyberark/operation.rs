use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationStatus {
    Success,
    Error,
    NotSupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SecretOperation {
    pub status: OperationStatus,
    pub message: String,
}
