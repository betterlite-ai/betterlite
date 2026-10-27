use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum Content {
    Text(String),
    Parts(Vec<ContentPart>),
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ContentPart {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub image_url: Option<ImageUrl>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum ImageUrl {
    Text(String),
    Object { url: String },
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: ToolFunction,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ToolFunction {
    pub name: String,
    pub arguments: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Message {
    pub role: String,
    pub content: Content,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PromptError {
    #[error("message {index} has unsupported role {role}")]
    Role { index: usize, role: String },
    #[error("message {index} must contain text")]
    Text { index: usize },
    #[error("message {index} has invalid tool-call arguments")]
    ToolArguments { index: usize },
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct OllamaPrompt {
    pub prompt: String,
    pub images: Vec<String>,
}

pub fn default_pt(messages: &[Message]) -> Result<String, PromptError> {
    let mut output = String::new();
    for (index, message) in messages.iter().enumerate() {
        if index > 0 {
            output.push(' ');
        }
        output.push_str(&message_text(message, index)?);
    }
    Ok(output)
}

pub fn map_system_message_pt(messages: &[Message]) -> Vec<Message> {
    let mut mapped = Vec::with_capacity(messages.len());
    let mut index = 0;
    while let Some(message) = messages.get(index) {
        if message.role == "system" {
            if let Some(next) = messages.get(index + 1) {
                if next.role == "user" || next.role == "assistant" {
                    let mut merged = next.clone();
                    merged.content = Content::Text(format!(
                        "{} {}",
                        content_text(&message.content),
                        content_text(&next.content)
                    ));
                    mapped.push(merged);
                    index += 2;
                    continue;
                }
                if next.role == "system" {
                    mapped.push(system_as_user(message));
                    index += 1;
                    continue;
                }
            } else {
                mapped.push(system_as_user(message));
                index += 1;
                continue;
            }
        } else {
            mapped.push(message.clone());
        }
        index += 1;
    }
    mapped
}

pub fn alpaca(messages: &[Message]) -> Result<String, PromptError> {
    custom(
        messages,
        "<s>",
        "",
        "</s>",
        &[
            ("system", "### Instruction:\n", "\n\n"),
            ("user", "### Instruction:\n", "\n\n"),
            ("assistant", "### Response:\n", "\n\n"),
        ],
    )
}
pub fn llama2(messages: &[Message]) -> Result<String, PromptError> {
    custom(
        messages,
        "<s>",
        "",
        "</s>",
        &[
            ("system", "[INST] <<SYS>>\n", "\n<</SYS>>\n [/INST]\n"),
            ("user", "[INST] ", " [/INST]\n"),
            ("assistant", "", "\n"),
        ],
    )
}
pub fn mistral(messages: &[Message]) -> Result<String, PromptError> {
    custom(
        messages,
        "",
        "<s>",
        "",
        &[
            ("system", "[INST] \n", " [/INST]\n"),
            ("user", "[INST] ", " [/INST]\n"),
            ("assistant", " ", "</s> "),
        ],
    )
}
pub fn granite(messages: &[Message]) -> Result<String, PromptError> {
    Ok(custom(
        messages,
        "",
        "",
        "",
        &[
            ("system", "<|system|>\n", "\n"),
            ("user", "<|user|>\n", "\n<|assistant|>\n"),
            ("assistant", "", "\n"),
        ],
    )?
    .trim()
    .to_owned())
}

pub fn falcon_instruct(messages: &[Message]) -> Result<String, PromptError> {
    let mut output = String::new();
    for (index, message) in messages.iter().enumerate() {
        let text = message_text(message, index)?;
        if message.role == "system" {
            output.push_str(&text);
        } else {
            output.push_str(&message.role);
            output.push(':');
            output.push_str(&text.replace("\r\n", "\n").replace("\n\n", "\n"));
            output.push_str("\n\n");
        }
    }
    Ok(output)
}
pub fn falcon_chat(messages: &[Message]) -> Result<String, PromptError> {
    labelled(
        messages,
        &[
            ("system", "System: "),
            ("assistant", "Falcon: "),
            ("user", "User: "),
        ],
    )
}
pub fn mpt(messages: &[Message]) -> Result<String, PromptError> {
    labelled_mpt(messages)
}
pub fn wizardcoder(messages: &[Message]) -> Result<String, PromptError> {
    labelled_double(
        messages,
        &[
            ("system", ""),
            ("user", "### Instruction:\n"),
            ("assistant", "### Response:\n"),
        ],
    )
}
pub fn phind(messages: &[Message]) -> Result<String, PromptError> {
    labelled_double(
        messages,
        &[
            ("system", "### System Prompt\n"),
            ("user", "### User Message\n"),
            ("assistant", "### Assistant\n"),
        ],
    )
}
pub fn claude(messages: &[Message]) -> Result<String, PromptError> {
    anthropic(messages, false)
}
pub fn claude_2_1(messages: &[Message]) -> Result<String, PromptError> {
    anthropic(messages, true)
}
pub fn titan(messages: &[Message]) -> Result<String, PromptError> {
    anthropic_titan(messages)
}

pub fn ollama(model: &str, messages: &[Message]) -> Result<OllamaPrompt, PromptError> {
    let mut prompt = String::new();
    let mut images = Vec::new();
    let mut index = 0;
    while index < messages.len() {
        let start = index;
        let mut user = String::new();
        while let Some(message) = messages.get(index) {
            if !["user", "tool", "function"].contains(&message.role.as_str()) {
                break;
            }
            collect_content(message, &mut user, &mut images);
            index += 1;
        }
        if !user.is_empty() {
            prompt.push_str("### User:\n");
            prompt.push_str(&user);
            prompt.push_str("\n\n");
        }
        let mut system = String::new();
        while let Some(message) = messages.get(index) {
            if message.role != "system" {
                break;
            }
            collect_content(message, &mut system, &mut images);
            index += 1;
        }
        if !system.is_empty() {
            prompt.push_str("### System:\n");
            prompt.push_str(&system);
            prompt.push_str("\n\n");
        }
        let mut assistant = String::new();
        while let Some(message) = messages.get(index) {
            if message.role != "assistant" {
                break;
            }
            collect_content(message, &mut assistant, &mut images);
            for call in &message.tool_calls {
                let arguments: serde_json::Value =
                    serde_json::from_str(&call.function.arguments)
                        .map_err(|_| PromptError::ToolArguments { index })?;
                let call_json = serde_json::json!([{"id": call.id, "type": "function", "function": {"name": call.function.name, "arguments": arguments}}]);
                assistant.push_str(&format!(
                    "Tool Calls: {}",
                    serde_json::to_string_pretty(&call_json)
                        .map_err(|_| PromptError::ToolArguments { index })?
                ));
            }
            index += 1;
        }
        if !assistant.is_empty() {
            prompt.push_str("### Assistant:\n");
            prompt.push_str(&assistant);
            prompt.push_str("\n\n");
        }
        if index == start {
            return Err(PromptError::Role {
                index,
                role: model.to_owned(),
            });
        }
    }
    Ok(OllamaPrompt { prompt, images })
}

fn custom(
    messages: &[Message],
    bos: &str,
    initial: &str,
    eos: &str,
    roles: &[(&str, &str, &str)],
) -> Result<String, PromptError> {
    let mut output = format!("{bos}{initial}");
    let mut bos_open = true;
    for (index, message) in messages.iter().enumerate() {
        let (_, pre, post) = roles
            .iter()
            .find(|(role, _, _)| *role == message.role)
            .ok_or_else(|| PromptError::Role {
                index,
                role: message.role.clone(),
            })?;
        if (message.role == "system" || message.role == "human") && !bos_open {
            output.push_str(bos);
            bos_open = true;
        }
        output.push_str(pre);
        output.push_str(&message_text(message, index)?);
        output.push_str(post);
        if message.role == "assistant" {
            output.push_str(eos);
            bos_open = false;
        }
    }
    Ok(output)
}
fn labelled(messages: &[Message], labels: &[(&str, &str)]) -> Result<String, PromptError> {
    let mut output = String::new();
    for (index, message) in messages.iter().enumerate() {
        let (_, label) = labels
            .iter()
            .find(|(role, _)| *role == message.role)
            .ok_or_else(|| PromptError::Role {
                index,
                role: message.role.clone(),
            })?;
        output.push_str(label);
        output.push_str(&message_text(message, index)?);
    }
    Ok(output)
}
fn labelled_double(messages: &[Message], labels: &[(&str, &str)]) -> Result<String, PromptError> {
    let mut output = String::new();
    for (index, message) in messages.iter().enumerate() {
        let (_, label) = labels
            .iter()
            .find(|(role, _)| *role == message.role)
            .ok_or_else(|| PromptError::Role {
                index,
                role: message.role.clone(),
            })?;
        output.push_str(label);
        output.push_str(&message_text(message, index)?);
        output.push_str("\n\n");
    }
    Ok(output)
}
fn labelled_mpt(messages: &[Message]) -> Result<String, PromptError> {
    let mut output = String::new();
    for (index, message) in messages.iter().enumerate() {
        let label = match message.role.as_str() {
            "system" => "system",
            "assistant" => "assistant",
            "user" => "user",
            role => {
                return Err(PromptError::Role {
                    index,
                    role: role.to_owned(),
                });
            }
        };
        output.push_str("<|im_start|>");
        output.push_str(label);
        output.push_str(&message_text(message, index)?);
        output.push_str("<|im_end|>\n");
    }
    Ok(output)
}
fn anthropic(messages: &[Message], v21: bool) -> Result<String, PromptError> {
    let mut output = String::new();
    for (index, message) in messages.iter().enumerate() {
        match message.role.as_str() {
            "user" => {
                output.push_str("\n\nHuman: ");
                output.push_str(&message_text(message, index)?);
            }
            "system" if v21 => output.push_str(&message_text(message, index)?),
            "system" => {
                output.push_str("\n\nHuman: <admin>");
                output.push_str(&message_text(message, index)?);
                output.push_str("</admin>");
            }
            "assistant" => {
                if v21
                    && index > 0
                    && messages
                        .get(index - 1)
                        .is_some_and(|item| item.role == "system")
                {
                    output.push_str("\n\nHuman: ");
                }
                output.push_str("\n\nAssistant: ");
                output.push_str(&message_text(message, index)?);
            }
            role => {
                return Err(PromptError::Role {
                    index,
                    role: role.to_owned(),
                });
            }
        }
    }
    if messages
        .last()
        .is_none_or(|message| message.role != "assistant")
    {
        output.push_str("\n\nAssistant: ");
    }
    Ok(output)
}
fn anthropic_titan(messages: &[Message]) -> Result<String, PromptError> {
    let mut output = String::new();
    for (index, message) in messages.iter().enumerate() {
        match message.role.as_str() {
            "user" => {
                output.push_str("\n\nUser: ");
                output.push_str(&message_text(message, index)?);
            }
            "system" => {
                output.push_str("\n\nUser: <admin>");
                output.push_str(&message_text(message, index)?);
                output.push_str("</admin>");
            }
            "assistant" => {
                output.push_str("\n\nBot: ");
                output.push_str(&message_text(message, index)?);
            }
            role => {
                return Err(PromptError::Role {
                    index,
                    role: role.to_owned(),
                });
            }
        }
    }
    if messages
        .last()
        .is_none_or(|message| message.role != "assistant")
    {
        output.push_str("\n\nBot: ");
    }
    Ok(output)
}
fn message_text(message: &Message, index: usize) -> Result<String, PromptError> {
    match &message.content {
        Content::Text(text) => Ok(text.clone()),
        Content::Parts(parts) => {
            let text: String = parts.iter().filter_map(|part| part.text.clone()).collect();
            if text.is_empty() {
                Err(PromptError::Text { index })
            } else {
                Ok(text)
            }
        }
    }
}
fn content_text(content: &Content) -> String {
    match content {
        Content::Text(text) => text.clone(),
        Content::Parts(parts) => parts.iter().filter_map(|part| part.text.clone()).collect(),
    }
}
fn system_as_user(message: &Message) -> Message {
    Message {
        role: "user".to_owned(),
        content: message.content.clone(),
        tool_calls: Vec::new(),
    }
}
fn collect_content(message: &Message, output: &mut String, images: &mut Vec<String>) {
    match &message.content {
        Content::Text(text) => output.push_str(text),
        Content::Parts(parts) => {
            for part in parts {
                if part.kind == "text" {
                    if let Some(text) = &part.text {
                        output.push_str(text);
                    }
                }
                if part.kind == "image_url" {
                    if let Some(image) = &part.image_url {
                        match image {
                            ImageUrl::Text(value) => images.push(value.clone()),
                            ImageUrl::Object { url } => images.push(url.clone()),
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod builtin_prompts_behavior_tests_behavior {
    use super::{Content, Message, alpaca, default_pt, map_system_message_pt};

    fn message(role: &str, content: &str) -> Message {
        Message {
            role: role.to_owned(),
            content: Content::Text(content.to_owned()),
            tool_calls: Vec::new(),
        }
    }

    #[test]
    fn default_joins_text_with_spaces() {
        let messages = [message("user", "one"), message("assistant", "two")];
        assert_eq!(default_pt(&messages), Ok("one two".to_owned()));
    }

    #[test]
    fn system_mapping_merges_into_next_turn() {
        let messages = [message("system", "rules"), message("user", "question")];
        let mapped = map_system_message_pt(&messages);
        assert_eq!(
            mapped.first().map(|item| &item.content),
            Some(&Content::Text("rules question".to_owned()))
        );
    }

    #[test]
    fn alpaca_preserves_delimiters() {
        let messages = [message("user", "question"), message("assistant", "answer")];
        assert_eq!(
            alpaca(&messages),
            Ok("<s>### Instruction:\nquestion\n\n### Response:\nanswer\n\n</s>".to_owned())
        );
    }
}
