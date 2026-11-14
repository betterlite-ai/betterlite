use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use std::collections::BTreeMap;
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
pub fn exception(fields: FailureFields) -> ProxyException {
    ProxyException::new(ProxyExceptionInit {
        message: fields.message,
        error_type: ERROR_TYPE.to_owned(),
        param: Some(fields.param),
        code: ExceptionCode::Status(fields.status),
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    })
}
