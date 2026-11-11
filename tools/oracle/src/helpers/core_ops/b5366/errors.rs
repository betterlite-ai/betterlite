use crate::helpers::HelperError;

pub(super) fn value_error(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("ValueError"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

pub(super) fn status_error(class: &str, status: u16, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: Some(status),
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
