use std::collections::BTreeMap;

use super::{ExceptionCode, ProxyException, ProxyExceptionInit};

#[derive(Debug, Clone, PartialEq, Eq)]
// ast-grep-ignore: no-long-names
pub struct ModelAccessDeniedProxyException {
    exception: ProxyException,
    internal_message: String,
}

// ast-grep-ignore: no-long-names
impl ModelAccessDeniedProxyException {
    #[must_use]
    pub fn new(
        message: String,
        internal_message: String,
        error_type: String,
        param: Option<String>,
        code: ExceptionCode,
    ) -> Self {
        Self {
            exception: ProxyException::new(ProxyExceptionInit {
                message,
                error_type,
                param,
                code,
                headers: BTreeMap::new(),
                openai_code: None,
                provider_specific_fields: BTreeMap::new(),
            }),
            internal_message,
        }
    }

    #[must_use]
    // ast-grep-ignore: no-one-line-functions
    pub fn sanitized_internal_message(&self) -> String {
        self.internal_message.replace(['\r', '\n'], "")
    }

    #[must_use]
    // ast-grep-ignore: no-one-line-functions
    pub const fn as_proxy_exception(&self) -> &ProxyException {
        &self.exception
    }
}
