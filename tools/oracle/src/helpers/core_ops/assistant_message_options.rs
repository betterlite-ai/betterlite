use betterlite_core::api::assistant_messages::options::{
    MessageOptions, OptionError, select_message_params,
};
use betterlite_prompts::arguments::json::Json;
use serde_json::Value;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

fn failure(error: &OptionError) -> Box<HelperError> {
    Box::new(HelperError {
        class: error.exception_name().to_owned(),
        status: error.status(),
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: error.litellm_message(),
    })
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = MessageOptions;

    fn run(input: &Self::Input) -> HelperResult {
        let selected = select_message_params(input).map_err(|error| failure(&error))?;
        let value: Value = serde_json::to_value(Json::Object(selected)).or_value_error()?;
        Ok(value)
    }
}
