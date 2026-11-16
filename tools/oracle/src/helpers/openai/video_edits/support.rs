use betterlite_oracle_helper_api::error::HelperError;

pub(super) fn helper_error(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from(class),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
