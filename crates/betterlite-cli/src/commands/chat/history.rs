use super::{ChatError, Message, Terminal};

const MAX_ASSISTANT_CHARS: usize = 100;
const HELP_LINES: [&str; 15] = [
    "Interactive Chat Commands:",
    "",
    "/help     - Show this help message",
    "/quit     - Exit the chat session (also /exit, /q)",
    "/clear    - Clear conversation history",
    "/history  - Show conversation history",
    "/model    - Switch to a different model",
    "/save <name> - Save conversation to file",
    "/load <name> - Load conversation from file",
    "",
    "Tips:",
    "- Your conversation history is maintained during the session",
    "- Use Ctrl+C to interrupt at any time",
    "- Responses are streamed in real-time",
    "- You can switch models mid-conversation with /model",
];

pub(super) fn write_welcome(
    terminal: &mut dyn Terminal,
    model: &str,
    temperature: f64,
    max_tokens: Option<i64>,
) -> Result<(), ChatError> {
    terminal.write_line("LiteLLM Interactive Chat")?;
    terminal.write_line(&format!("Model: {model}"))?;
    terminal.write_line(&format!("Temperature: {temperature}"))?;
    let tokens = max_tokens.map_or_else(|| String::from("unlimited"), |value| value.to_string());
    terminal.write_line(&format!("Max Tokens: {tokens}"))?;
    terminal.write_line("")?;
    terminal.write_line(
        "Type your messages and press Enter. Type '/quit' or '/exit' to end the session.",
    )?;
    terminal.write_line("Type '/help' for more commands.")?;
    Ok(())
}

pub(super) fn write_help(terminal: &mut dyn Terminal) -> Result<(), ChatError> {
    for line in HELP_LINES {
        terminal.write_line(line)?;
    }
    Ok(())
}

pub(super) fn write_history(
    terminal: &mut dyn Terminal,
    messages: &[Message],
) -> Result<(), ChatError> {
    if messages.is_empty() {
        terminal.write_line("No conversation history.")?;
        return Ok(());
    }
    terminal.write_line("Conversation History")?;
    for (index, message) in (1_usize..).zip(messages) {
        match message.role.as_str() {
            "system" => terminal.write_line(&format!("{index}. System: {}", message.content))?,
            "user" => terminal.write_line(&format!("{index}. You: {}", message.content))?,
            "assistant" => terminal.write_line(&format!(
                "{index}. Assistant: {}",
                assistant_preview(&message.content)
            ))?,
            _ => {}
        }
    }
    Ok(())
}

fn assistant_preview(content: &str) -> String {
    let mut remaining = content.chars();
    let head: String = remaining.by_ref().take(MAX_ASSISTANT_CHARS).collect();
    if remaining.next().is_some() {
        return format!("{head}...");
    }
    head
}
