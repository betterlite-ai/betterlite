use super::{Content, Message, PromptError, bounds, text};

pub fn map_system_message_pt(messages: &[Message]) -> Result<Vec<Message>, PromptError> {
    bounds::check(messages)?;
    let mut output = Vec::new();
    let mut turns = messages.iter().enumerate().peekable();
    while let Some((index, message)) = turns.next() {
        if message.role != "system" {
            output.push(message.clone());
            continue;
        }
        match turns.peek() {
            Some((next_index, next)) if next.role == "user" || next.role == "assistant" => {
                let mut merged = (*next).clone();
                merged.content = Content::Text(format!(
                    "{} {}",
                    text::strict(message, index)?,
                    text::strict(next, *next_index)?
                ));
                output.push(merged);
                turns.next();
            }
            Some((_, next)) if next.role != "system" => {}
            Some(_) | None => output.push(Message {
                role: "user".to_owned(),
                content: message.content.clone(),
                tool_calls: Vec::new(),
            }),
        }
    }
    Ok(output)
}

mod tests;
