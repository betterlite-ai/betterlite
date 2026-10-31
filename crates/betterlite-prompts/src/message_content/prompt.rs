use crate::arguments::json::{Json, Object};

use super::{Content, ContentError, Message};

pub fn to_messages(prompt: &Json) -> Result<Vec<Message>, ContentError> {
    let prompt_type = match prompt {
        Json::Text(value) if !value.is_empty() => {
            return Ok(vec![text_message(value.clone())]);
        }
        Json::Text(_) => "str",
        Json::List(entries) => {
            let strings = entries
                .iter()
                .filter_map(Json::as_str)
                .filter(|entry| !entry.is_empty())
                .collect::<Vec<_>>();
            if strings.len() != entries.len() || strings.is_empty() {
                return Err(ContentError::InvalidPrompt {
                    prompt_type: "list".to_owned(),
                });
            }
            let mut messages = Vec::with_capacity(strings.len());
            for entry in strings {
                messages.push(text_message(entry.to_owned()));
            }
            return Ok(messages);
        }
        // Python's `type(prompt).__name__` names "int" and "float" separately.
        Json::Number(number) if number.is_f64() => "float",
        Json::Number(_) => "int",
        Json::Bool(_) => "bool",
        Json::Null => "NoneType",
        Json::Object(_) => "dict",
    };
    Err(ContentError::InvalidPrompt {
        prompt_type: prompt_type.to_owned(),
    })
}

fn text_message(content: String) -> Message {
    let provider_specific_fields = Object::new();
    Message {
        role: "user".to_owned(),
        content: Some(Content::Text(content)),
        name: None,
        provider_specific_fields,
    }
}

mod tests;
