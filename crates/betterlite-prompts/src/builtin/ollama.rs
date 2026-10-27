use super::{Content, Field, Message, OllamaPrompt, PromptError, bounds, text};
mod calls;
use std::fmt::Write as _;

pub fn ollama(model: &str, messages: &[Message]) -> Result<OllamaPrompt, PromptError> {
    bounds::check(messages)?;
    if model.len() > bounds::OUTPUT_BYTES {
        return Err(PromptError::Limit);
    }
    let mut prompt = String::new();
    let mut images = Vec::new();
    let mut turns = messages.iter().enumerate().peekable();
    while let Some((index, message)) = turns.peek() {
        let (roles, label): (&[&str], &str) = match message.role.as_str() {
            "user" | "tool" | "function" => (&["user", "tool", "function"], "User"),
            "system" => (&["system"], "System"),
            "assistant" => (&["assistant"], "Assistant"),
            role => {
                return Err(PromptError::Role {
                    index: *index,
                    role: format!("{role} ({model})"),
                });
            }
        };
        let mut content = String::new();
        while let Some((_, turn)) = turns.next_if(|(_, turn)| roles.contains(&turn.role.as_str())) {
            if label == "User" {
                collect(turn, &mut content, &mut images)?;
            } else {
                content.push_str(&text::joined(&turn.content));
            }
            if label == "Assistant" && !turn.tool_calls.is_empty() {
                content.push_str("Tool Calls: ");
                content.push_str(&calls::render(&turn.tool_calls)?);
            }
        }
        if !content.is_empty() {
            write!(prompt, "### {label}:\n{content}\n\n")?;
        }
    }
    Ok(OllamaPrompt {
        prompt: bounds::finish(prompt)?,
        images,
    })
}

fn collect(
    message: &Message,
    output: &mut String,
    images: &mut Vec<String>,
) -> Result<(), PromptError> {
    if let Content::Text(text) = &message.content {
        output.push_str(text);
    }
    let Content::Parts(parts) = &message.content else {
        return Ok(());
    };
    for part in parts {
        collect_part(part, output, images)?;
    }
    Ok(())
}

fn collect_part(
    part: &super::ContentPart,
    output: &mut String,
    images: &mut Vec<String>,
) -> Result<(), PromptError> {
    match part.text("type") {
        Some("text") => output.push_str(
            part.text("text")
                .ok_or(PromptError::MissingPart { field: "text" })?,
        ),
        Some("image_url") => match part.fields.get("image_url") {
            Some(Field::Text(url)) => images.push(url.clone()),
            Some(Field::Object(fields)) => match fields.get("url") {
                Some(Field::Text(url)) => images.push(url.clone()),
                _ => return Err(PromptError::MissingPart { field: "url" }),
            },
            None => return Err(PromptError::MissingPart { field: "image_url" }),
            Some(_) => {}
        },
        Some(_) | None => {}
    }
    Ok(())
}

mod tests;
