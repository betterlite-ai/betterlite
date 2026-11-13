use crate::helpers::HelperError;

pub(super) fn failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "BatchEnqueuedRefundError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
