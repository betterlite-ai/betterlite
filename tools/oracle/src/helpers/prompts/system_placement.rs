use crate::helpers::{Helper, HelperError, HelperResult};
use betterlite_prompts::message_content::Message;
use betterlite_prompts::system_placement::anthropic_system::anthropic_system_messages;
use betterlite_prompts::system_placement::text_parts::system_message_as_user;
use betterlite_prompts::system_placement::{
    place_mid_conversation_system, split_leading_system_run,
};
use serde::Deserialize;
use serde_json::to_value;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", deny_unknown_fields)]
pub(crate) enum Input {
    #[serde(rename = "split_leading_system_run")]
    SplitLeading { messages: Vec<Message> },
    #[serde(rename = "anthropic_system_messages")]
    AnthropicSystem { message: Message },
    #[serde(rename = "system_message_as_user")]
    AsUser { message: Message },
    #[serde(rename = "place_mid_conversation_system")]
    Place {
        messages: Vec<Message>,
        #[serde(rename = "supports_mid_conversation_system")]
        supports_mid_convo_system: bool,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::SplitLeading { messages } => ok_value(split_leading_system_run(messages)),
            Input::AnthropicSystem { message } => ok_value(anthropic_system_messages(message)),
            Input::AsUser { message } => ok_value(system_message_as_user(message)),
            Input::Place {
                messages,
                supports_mid_convo_system,
            } => ok_value(place_mid_conversation_system(
                messages,
                *supports_mid_convo_system,
            )),
        }
    }
}

fn ok_value<Payload>(payload: Payload) -> HelperResult
where
    Payload: serde::Serialize,
{
    match to_value(payload) {
        Ok(json) => Ok(json),
        Err(error) => Err(classified_error(error.to_string())),
    }
}

fn classified_error(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "SerializationError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

mod tests;
