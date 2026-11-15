use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct ErrorDetail {
    #[serde(rename = "type")]
    pub error_type: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ErrorBody {
    pub error: ErrorDetail,
}

#[derive(Clone, Debug)]
pub struct ErrorReply {
    pub status: u16,
    pub body: ErrorBody,
}

#[derive(Clone, Debug)]
pub struct Failure {
    pub error_type: String,
    pub message: String,
    pub status: Option<u16>,
}
