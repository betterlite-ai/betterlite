pub mod error;

use indexmap::IndexMap;

use crate::decisions::Json;
use error::BillingTextError;

mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptCallType {
    Acompletion,
    Completion,
    TextCompletion,
    Embedding,
    ImageGeneration,
    AudioTranscription,
    Moderation,
}

pub struct FormattedPromptData<'text> {
    pub messages: Option<&'text [Json]>,
    pub prompt: Option<&'text Json>,
    pub input: Option<&'text Json>,
}

pub struct ResponseText<'text> {
    pub event: Option<EventText<'text>>,
    pub choices: &'text [ChoiceText<'text>],
}

pub struct EventText<'text> {
    pub event_type: &'text str,
    pub output: Option<&'text [Json]>,
    pub delta: Option<&'text Json>,
}

pub enum ChoiceText<'text> {
    Message { content: Option<&'text Json> },
    Delta { content: Option<&'text Json> },
}

#[must_use]
pub fn count_characters(text: &str) -> usize {
    // Python counts str.isspace() characters, which also covers the U+001C..U+001F separators
    text.chars()
        .filter(|character| {
            !(character.is_whitespace() || matches!(*character, '\u{1c}'..='\u{1f}'))
        })
        .count()
}

const fn as_object<'value>(
    value: &'value Json,
    field: &'static str,
) -> Result<&'value IndexMap<String, Json>, BillingTextError> {
    let Json::Object(fields) = value else {
        return Err(BillingTextError::NotObject { field });
    };
    Ok(fields)
}

fn as_text<'value>(
    value: &'value Json,
    field: &'static str,
) -> Result<&'value str, BillingTextError> {
    if let Json::Text(text) = value {
        return Ok(text);
    }
    Err(BillingTextError::NotText { field })
}

pub fn get_formatted_prompt(
    data: &FormattedPromptData<'_>,
    call_type: PromptCallType,
) -> Result<String, BillingTextError> {
    match call_type {
        PromptCallType::Acompletion | PromptCallType::Completion => chat_prompt(data.messages),
        PromptCallType::TextCompletion | PromptCallType::ImageGeneration => {
            required_text(data.prompt)
        }
        PromptCallType::Embedding | PromptCallType::Moderation => input_prompt(data.input),
        PromptCallType::AudioTranscription => optional_text(data.prompt),
    }
}

fn required_text(prompt: Option<&Json>) -> Result<String, BillingTextError> {
    let Some(value) = prompt.filter(|value| !matches!(value, Json::Null)) else {
        return Err(BillingTextError::Missing { field: "prompt" });
    };
    Ok(as_text(value, "prompt")?.to_owned())
}

fn optional_text(prompt: Option<&Json>) -> Result<String, BillingTextError> {
    match prompt {
        None | Some(Json::Null) => Ok(String::new()),
        Some(value) => Ok(as_text(value, "prompt")?.to_owned()),
    }
}

fn input_prompt(input: Option<&Json>) -> Result<String, BillingTextError> {
    match input {
        Some(Json::Text(text)) => Ok(text.clone()),
        Some(Json::List(items)) => {
            let mut prompt = String::new();
            for item in items {
                prompt.push_str(as_text(item, "input")?);
            }
            Ok(prompt)
        }
        None | Some(Json::Null) => Err(BillingTextError::Missing { field: "input" }),
        Some(Json::Bool(_) | Json::Number(_) | Json::Object(_)) => {
            Err(BillingTextError::NotTextOrList { field: "input" })
        }
    }
}

fn chat_prompt(messages: Option<&[Json]>) -> Result<String, BillingTextError> {
    let found = messages.ok_or(BillingTextError::Missing { field: "messages" })?;
    let mut prompt = String::new();
    for message in found {
        let fields = as_object(message, "messages")?;
        append_content(&mut prompt, fields.get("content"))?;
        append_tool_calls(&mut prompt, fields.get("tool_calls"))?;
    }
    Ok(prompt)
}

fn append_content(prompt: &mut String, content: Option<&Json>) -> Result<(), BillingTextError> {
    let Some(value) = content else {
        return Ok(());
    };
    if matches!(value, Json::Null) {
        return Ok(());
    }
    if let Json::Text(text) = value {
        prompt.push_str(text);
        return Ok(());
    }
    if let Json::List(parts) = value {
        return append_text_parts(prompt, parts);
    }
    Err(BillingTextError::NotTextOrList {
        field: "messages.content",
    })
}

fn append_text_parts(prompt: &mut String, parts: &[Json]) -> Result<(), BillingTextError> {
    for part in parts {
        let fields = as_object(part, "messages.content")?;
        let part_type = fields.get("type").ok_or(BillingTextError::Missing {
            field: "messages.content.type",
        })?;
        if as_text(part_type, "messages.content.type")? != "text" {
            continue;
        }
        let part_text = fields.get("text").ok_or(BillingTextError::Missing {
            field: "messages.content.text",
        })?;
        prompt.push_str(as_text(part_text, "messages.content.text")?);
    }
    Ok(())
}

fn append_tool_calls(prompt: &mut String, calls: Option<&Json>) -> Result<(), BillingTextError> {
    let Some(value) = calls.filter(|value| !matches!(value, Json::Null)) else {
        return Ok(());
    };
    let Json::List(entries) = value else {
        return Err(BillingTextError::NotList {
            field: "messages.tool_calls",
        });
    };
    for call in entries {
        let fields = as_object(call, "messages.tool_calls")?;
        let Some(function) = fields.get("function") else {
            continue;
        };
        let function_fields = as_object(function, "messages.tool_calls.function")?;
        let Some(arguments) = function_fields.get("arguments") else {
            return Err(BillingTextError::Missing {
                field: "messages.tool_calls.function.arguments",
            });
        };
        prompt.push_str(as_text(
            arguments,
            "messages.tool_calls.function.arguments",
        )?);
    }
    Ok(())
}

pub fn get_response_string(response: &ResponseText<'_>) -> Result<String, BillingTextError> {
    if let Some(event) = &response.event {
        if let Some(output) = event.output {
            let mut text = String::new();
            for item in output {
                append_output_item(&mut text, item)?;
            }
            return Ok(text);
        }
        if let Some(delta) = event.delta
            && (event.event_type.contains("text.delta")
                || event.event_type.contains("output_text.delta"))
        {
            return match delta {
                Json::Null => Ok(String::new()),
                Json::Text(found) => Ok(found.clone()),
                Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                    Err(BillingTextError::NotText { field: "delta" })
                }
            };
        }
    }
    let mut text = String::new();
    for choice in response.choices {
        let content = match choice {
            ChoiceText::Message { content } | ChoiceText::Delta { content } => content,
        };
        match content {
            None | Some(Json::Null) => {}
            Some(Json::Text(part)) => text.push_str(part),
            Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
                return Err(BillingTextError::NotText { field: "content" });
            }
        }
    }
    Ok(text)
}

fn append_output_item(text: &mut String, item: &Json) -> Result<(), BillingTextError> {
    let Json::Object(fields) = item else {
        return Ok(());
    };
    if let Some(content) = fields.get("content") {
        let Json::List(parts) = content else {
            return Err(BillingTextError::NotList {
                field: "output.content",
            });
        };
        for part in parts {
            let Json::Object(part_fields) = part else {
                continue;
            };
            match part_fields.get("text") {
                None => {}
                Some(Json::Text(found)) => text.push_str(found),
                Some(
                    Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
                ) => {
                    return Err(BillingTextError::NotText {
                        field: "output.content.text",
                    });
                }
            }
        }
        return Ok(());
    }
    if let Some(value) = fields.get("text") {
        text.push_str(as_text(value, "output.text")?);
    }
    Ok(())
}
