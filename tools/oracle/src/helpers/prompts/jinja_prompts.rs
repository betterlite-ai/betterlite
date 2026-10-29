use crate::helpers::{Helper, HelperError, HelperResult};
use betterlite_prompts::builtin::Message;
use betterlite_prompts::jinja::{LocalTemplate, RenderError, TokenValue, extract_token_value, hf_chat_template};
use chrono::{TimeZone, Utc};
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub(crate) enum Input {
    Render {
        chat_template: String,
        #[serde(default)]
        bos_token: Option<TokenValue>,
        #[serde(default)]
        eos_token: Option<TokenValue>,
        messages: Vec<Message>,
    },
    ExtractToken {
        token: Option<TokenValue>,
    },
}

pub(crate) struct Op;
impl Helper for Op {
    type Input = Input;
    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Render {
                chat_template,
                bos_token,
                eos_token,
                messages,
            } => {
                let template = LocalTemplate {
                    chat_template: chat_template.clone(),
                    bos_token: bos_token.clone(),
                    eos_token: eos_token.clone(),
                };
                let now = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
                hf_chat_template(&template, messages, now)
                    .map(Value::from)
                    .map_err(|error| failure(&error))
            }
            Input::ExtractToken { token } => Ok(Value::from(extract_token_value(token.clone()))),
        }
    }
}

fn failure(error: &RenderError) -> Box<HelperError> {
    let class = match error {
        RenderError::Compile { .. } => "TemplateSyntaxError",
        RenderError::Render { .. } => "Exception",
        RenderError::TemplateTooLarge | RenderError::OutputTooLarge => "LimitError",
    };
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

mod tests;
