use crate::helpers::{Helper, HelperError, HelperResult};
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::{ContentPart, Message};
use betterlite_prompts::message_fields::file_migration::migrate_file_to_image_url;
use betterlite_prompts::message_fields::image_hoist::{
    extract_images_from_message, hoist_tool_message_images,
};
use betterlite_prompts::message_fields::internal_fields::strip_internal_message_fields;
use betterlite_prompts::message_fields::last_user::{get_last_user_message, set_last_user_message};
use betterlite_prompts::message_fields::nested_filter::filter_value_from_dict;
use betterlite_prompts::message_fields::prefix::convert_prefix_messages;
use betterlite_prompts::message_fields::system_prompt::add_system_prompt_to_messages;
use betterlite_prompts::message_fields::system_runs::{
    merge_adjacent_system_messages, system_messages_first,
};
use betterlite_prompts::message_fields::tool_reference::drop_tool_reference_parts;
use serde::Deserialize;
use serde_json::to_value;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", deny_unknown_fields)]
pub(crate) enum Input {
    #[serde(rename = "strip_litellm_internal_message_fields")]
    StripInternal { message: Message },
    #[serde(rename = "filter_value_from_dict")]
    FilterDict {
        dictionary: Object,
        key: String,
        #[serde(default)]
        depth: u32,
    },
    #[serde(rename = "migrate_file_to_image_url")]
    MigrateFile { part: ContentPart },
    #[serde(rename = "get_last_user_message")]
    LastUserGet { messages: Vec<Message> },
    #[serde(rename = "set_last_user_message")]
    LastUserSet {
        messages: Vec<Message>,
        content: String,
    },
    #[serde(rename = "add_system_prompt_to_messages")]
    AddSystemPrompt {
        messages: Vec<Message>,
        system_prompt: String,
        #[serde(default)]
        merge_with_first_system: bool,
    },
    #[serde(rename = "convert_prefix_message_to_non_prefix_messages")]
    ConvertPrefix { messages: Vec<Message> },
    #[serde(rename = "extract_images_from_message")]
    ExtractImages { message: Message },
    #[serde(rename = "hoist_images_from_tool_messages")]
    HoistImages { messages: Vec<Message> },
    #[serde(rename = "drop_tool_reference_parts_from_tool_messages")]
    DropToolRef { messages: Vec<Message> },
    #[serde(rename = "system_messages_first")]
    SystemFirst { messages: Vec<Message> },
    #[serde(rename = "merge_consecutive_system_messages")]
    MergeSystem { messages: Vec<Message> },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::StripInternal { message } => ok_value(strip_internal_message_fields(message)),
            Input::FilterDict {
                dictionary,
                key,
                depth,
            } => ok_value(filter_value_from_dict(dictionary, key, *depth)),
            Input::MigrateFile { part } => match migrate_file_to_image_url(part) {
                Ok(image_part) => ok_value(image_part),
                Err(error) => Err(migration_error(&error)),
            },
            Input::LastUserGet { messages } => ok_value(get_last_user_message(messages)),
            Input::LastUserSet { messages, content } => {
                ok_value(set_last_user_message(messages, content))
            }
            Input::AddSystemPrompt {
                messages,
                system_prompt,
                merge_with_first_system,
            } => ok_value(add_system_prompt_to_messages(
                messages,
                system_prompt,
                *merge_with_first_system,
            )),
            Input::ConvertPrefix { messages } => ok_value(convert_prefix_messages(messages)),
            Input::ExtractImages { message } => ok_value(extract_images_from_message(message)),
            Input::HoistImages { messages } => ok_value(hoist_tool_message_images(messages)),
            Input::DropToolRef { messages } => ok_value(drop_tool_reference_parts(messages)),
            Input::SystemFirst { messages } => ok_value(system_messages_first(messages)),
            Input::MergeSystem { messages } => ok_value(merge_adjacent_system_messages(messages)),
        }
    }
}

fn ok_value<Payload>(payload: Payload) -> HelperResult
where
    Payload: serde::Serialize,
{
    match to_value(payload) {
        Ok(json) => Ok(json),
        Err(error) => Err(classified_error("SerializationError", error.to_string())),
    }
}

fn classified_error(class: &str, message: String) -> Box<HelperError> {
    let error_class = class.to_owned();
    Box::new(HelperError {
        class: error_class,
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn migration_error(
    error: &betterlite_prompts::message_fields::file_migration::FileMigrationError,
) -> Box<HelperError> {
    use betterlite_prompts::message_fields::file_migration::FileMigrationError;
    let (class, status) = match error {
        FileMigrationError::MissingFile => ("BadRequestError", Some(400)),
        FileMigrationError::MissingFileReference => ("ValueError", None),
    };
    Box::new(HelperError {
        class: class.to_owned(),
        status,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
