use crate::message_content::Message;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageKind {
    System,
    Tool,
    User,
    Other,
}

#[must_use]
pub fn kind(message: &Message) -> MessageKind {
    match message.role.as_str() {
        "system" => MessageKind::System,
        "tool" | "function" => MessageKind::Tool,
        "user" => MessageKind::User,
        _ => MessageKind::Other,
    }
}

mod tests;
