use betterlite_core::api::assistant_messages::options::{
    MessageOptions, OptionError, get_optional_params_add_message,
};
use betterlite_prompts::arguments::json::{Json, Object};
use serde::Deserialize;
use serde_json::Value;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

// the selector's named arguments plus the caller's extra keywords, exactly as add_message passes them
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    custom_llm_provider: String,
    drop_params: bool,
    #[serde(default)]
    role: Option<Json>,
    #[serde(default)]
    content: Option<Json>,
    #[serde(default)]
    attachments: Option<Json>,
    #[serde(default)]
    metadata: Option<Json>,
    #[serde(default)]
    kwargs: Object,
}

fn failure(error: &OptionError) -> Box<HelperError> {
    Box::new(HelperError {
        class: error.python_class().to_owned(),
        status: error.status(),
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: error.python_message(),
    })
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let options = MessageOptions {
            role: input.role.clone(),
            content: input.content.clone(),
            attachments: input.attachments.clone(),
            metadata: input.metadata.clone(),
            provider: input.custom_llm_provider.clone(),
            kwargs: input.kwargs.clone(),
        };
        let selected = get_optional_params_add_message(&options, input.drop_params)
            .map_err(|error| failure(&error))?;
        let value: Value = serde_json::to_value(Json::Object(selected)).or_value_error()?;
        Ok(value)
    }
}
