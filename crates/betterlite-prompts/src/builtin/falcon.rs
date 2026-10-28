use super::{Message, PromptError, bounds, text};
use std::fmt::Write as _;

pub fn falcon_instruct(messages: &[Message]) -> Result<String, PromptError> {
    bounds::check(messages)?;
    let mut output = String::new();
    for (index, message) in messages.iter().enumerate() {
        let content = text::strict(message, index)?;
        if message.role == "system" {
            output.push_str(content);
        } else {
            write!(
                output,
                "{}:{}\n\n",
                message.role,
                content.replace("\r\n", "\n").replace("\n\n", "\n")
            )?;
        }
    }
    bounds::finish(output)
}
