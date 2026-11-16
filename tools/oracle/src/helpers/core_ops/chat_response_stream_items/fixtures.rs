mod assembly;
mod sinks;

use betterlite_oracle_helper_api::error::HelperError;

pub(crate) use assembly::AssemblyContext;

pub(crate) fn failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
