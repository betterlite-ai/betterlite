use super::bounds;
use crate::builtin::{Calls, Content, ContentPart, Field, Message};
use crate::error::FactoryError;
use betterlite_logging::analytics::python::repr;
use indexmap::IndexMap;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::slice;

#[derive(Clone, Debug, Default, Deserialize)]
pub struct RoleTemplate {
    #[serde(default)]
    pub pre_message: String,
    #[serde(default)]
    pub post_message: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct CustomPromptTemplate {
    #[serde(default)]
    pub roles: BTreeMap<String, RoleTemplate>,
    #[serde(default)]
    pub initial_prompt_value: String,
    #[serde(default)]
    pub final_prompt_value: String,
    #[serde(default)]
    pub bos_token: String,
    #[serde(default)]
    pub eos_token: String,
}

pub fn custom_prompt(
    template: &CustomPromptTemplate,
    messages: &[Message],
) -> Result<String, FactoryError> {
    bounds::check(messages)?;
    let mut prompt = format!("{}{}", template.bos_token, template.initial_prompt_value);
    let mut bos_open = true;
    for message in messages {
        if matches!(message.role.as_str(), "system" | "human") && !bos_open {
            prompt.push_str(&template.bos_token);
            bos_open = true;
        }
        render_message(&mut prompt, template, message);
        if message.role == "assistant" {
            prompt.push_str(&template.eos_token);
            bos_open = false;
        }
    }
    prompt.push_str(&template.final_prompt_value);
    bounds::finish(prompt)
}

fn render_message(prompt: &mut String, template: &CustomPromptTemplate, message: &Message) {
    let role_template = template.roles.get(&message.role);
    let pre = role_template.map_or("", |entry| entry.pre_message.as_str());
    let post = role_template.map_or("", |entry| entry.post_message.as_str());
    match &message.content {
        Content::Text(text) => {
            prompt.push_str(pre);
            prompt.push_str(text);
            prompt.push_str(post);
        }
        Content::Parts(parts) => {
            prompt.push_str(pre);
            for part in parts {
                if let Some(Field::Text(text)) = part.fields.get("text") {
                    prompt.push_str(text);
                }
            }
            prompt.push_str(post);
        }
        Content::Null => {}
    }
}

pub fn function_call_prompt(
    messages: &[Message],
    functions: &[Field],
) -> Result<Vec<Message>, FactoryError> {
    bounds::check(messages)?;
    let mut function_prompt = String::from(
        "Produce JSON OUTPUT ONLY! Adhere to this format {\"name\": \"function_name\", \
         \"arguments\":{\"argument_name\": \"argument_value\"}} The following functions are \
         available to you:",
    );
    for function in functions {
        function_prompt.push('\n');
        function_prompt.push_str(&repr(function)?);
        function_prompt.push('\n');
    }

    let mut output = messages.to_vec();
    let mut added = false;
    for (index, message) in output.iter_mut().enumerate() {
        if !message.role.contains("system") {
            continue;
        }
        append_function_prompt(message, &function_prompt, index)?;
        added = true;
    }
    if !added {
        output.push(Message {
            role: "system".to_owned(),
            content: Content::Text(function_prompt),
            tool_calls: Calls::Absent,
            extra: IndexMap::new(),
        });
    }
    Ok(output)
}

fn append_function_prompt(
    message: &mut Message,
    function_prompt: &str,
    index: usize,
) -> Result<(), FactoryError> {
    match &mut message.content {
        Content::Text(text) => {
            text.push(' ');
            text.push_str(function_prompt);
            Ok(())
        }
        Content::Parts(parts) => {
            let mut fields = IndexMap::new();
            fields.insert("type".to_owned(), Field::Text("text".to_owned()));
            fields.insert(
                "text".to_owned(),
                Field::Text(format!(" {function_prompt}")),
            );
            parts.push(ContentPart { fields });
            Ok(())
        }
        Content::Null => Err(FactoryError::Append { index }),
    }
}

pub fn response_schema_prompt(
    response_schema: &Field,
    template: Option<&CustomPromptTemplate>,
) -> Result<String, FactoryError> {
    match template {
        Some(custom_template) => {
            let message = Message {
                role: "user".to_owned(),
                content: Content::Text(repr(response_schema)?),
                tool_calls: Calls::Absent,
                extra: IndexMap::new(),
            };
            let without_tokens = CustomPromptTemplate {
                bos_token: String::new(),
                eos_token: String::new(),
                ..custom_template.clone()
            };
            custom_prompt(&without_tokens, slice::from_ref(&message))
        }
        None => default_response_schema_prompt(response_schema),
    }
}

pub fn default_response_schema_prompt(response_schema: &Field) -> Result<String, FactoryError> {
    let schema = repr(response_schema)?;
    Ok(format!(
        "Use this JSON schema: \n    ```json \n    {schema}\n    ```"
    ))
}

mod tests;
