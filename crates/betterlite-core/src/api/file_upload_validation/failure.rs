pub(super) const ERROR_TYPE: &str = "invalid_request_error";
pub(super) const INVALID_REQUEST: i64 = 400;
pub(super) const FILE_TOO_LARGE: i64 = 413;
pub(super) const FILE_PARAM: &str = "file";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureFields {
    pub message: String,
    pub status: i64,
    pub param: String,
}
