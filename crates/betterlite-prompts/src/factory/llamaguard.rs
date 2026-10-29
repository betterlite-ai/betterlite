use super::Output;
use crate::builtin::Message;
use crate::error::FactoryError;
use crate::jinja::{self, LocalTemplate};
use chrono::{DateTime, Utc};

const TEMPLATE: &str = include_str!("fixtures/llamaguard_7b_chat_template.jinja");

pub(super) fn render(messages: &[Message], now: DateTime<Utc>) -> Result<Output, FactoryError> {
    let template = LocalTemplate {
        chat_template: TEMPLATE.to_owned(),
        bos_token: None,
        eos_token: None,
    };
    jinja::hf_chat_template(&template, messages, now)
        .map(Output::Text)
        .map_err(FactoryError::from)
}

mod tests;
