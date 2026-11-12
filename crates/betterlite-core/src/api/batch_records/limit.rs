use std::collections::BTreeMap;

use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};

use super::BatchRecordsError;

pub(super) fn validate_batch_list_limit(limit: Option<i64>) -> Result<(), BatchRecordsError> {
    let Some(requested) = limit else {
        return Ok(());
    };
    let (bound, expected, openai_code) = match requested {
        0..=100 => return Ok(()),
        i64::MIN..0 => ("below minimum", ">= 0", "integer_below_min_value"),
        101.. => ("above maximum", "<= 100", "integer_above_max_value"),
    };
    Err(BatchRecordsError::ListLimit(Box::new(ProxyException::new(
        ProxyExceptionInit {
            message: format!(
                "Invalid 'limit': integer {bound} value. Expected a value {expected}, but got {requested} instead."
            ),
            error_type: "invalid_request_error".to_owned(),
            param: Some("limit".to_owned()),
            code: ExceptionCode::Status(400),
            openai_code: Some(openai_code.to_owned()),
            headers: BTreeMap::new(),
            provider_specific_fields: BTreeMap::new(),
        },
    ))))
}
