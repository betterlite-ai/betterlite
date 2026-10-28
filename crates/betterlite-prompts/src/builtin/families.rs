use super::{Content, Message, PromptError, bounds, text};

pub(super) fn custom(
    messages: &[Message],
    tokens: (&str, &str, &str),
    roles: &[(&str, &str, &str)],
    trim: bool,
) -> Result<String, PromptError> {
    bounds::check(messages)?;
    let (bos, initial, eos) = tokens;
    let mut output = format!("{bos}{initial}");
    let mut bos_open = true;
    for message in messages {
        let (_, pre, post) = roles
            .iter()
            .find(|(role, _, _)| *role == message.role)
            .copied()
            .unwrap_or(("", "", ""));
        if (message.role == "system" || message.role == "human") && !bos_open {
            output.push_str(bos);
            bos_open = true;
        }
        if !matches!(message.content, Content::Null) {
            output.push_str(pre);
            output.push_str(&text::joined(&message.content));
            output.push_str(post);
        }
        if message.role == "assistant" {
            output.push_str(eos);
            bos_open = false;
        }
    }
    bounds::finish(if trim {
        output.trim().to_owned()
    } else {
        output
    })
}

pub(super) fn labelled(
    messages: &[Message],
    labels: &[(&str, &str)],
    suffix: &str,
) -> Result<String, PromptError> {
    bounds::check(messages)?;
    let mut output = String::new();
    for (index, message) in messages.iter().enumerate() {
        if let Some((_, label)) = labels.iter().find(|(role, _)| *role == message.role) {
            output.push_str(label);
            output.push_str(text::strict(message, index)?);
            output.push_str(suffix);
        }
    }
    bounds::finish(output)
}

#[derive(Clone, Copy)]
pub(super) enum Style {
    Claude,
    Claude21,
    Titan,
}

pub(super) fn anthropic(messages: &[Message], style: Style) -> Result<String, PromptError> {
    bounds::check(messages)?;
    let final_message = messages.last().ok_or(PromptError::Empty)?;
    let assistant = match style {
        Style::Claude | Style::Claude21 => "\n\nAssistant: ",
        Style::Titan => "\n\nBot: ",
    };
    let mut output = String::new();
    let mut previous = None;
    for (index, message) in messages.iter().enumerate() {
        let role = message.role.as_str();
        let delimiters = framing(style, role, (index, previous), &mut output);
        let Some((prefix, suffix)) = delimiters else {
            previous = Some(role);
            continue;
        };
        output.push_str(prefix);
        output.push_str(&text::display(&message.content)?);
        output.push_str(suffix);
        previous = Some(role);
    }
    if final_message.role != "assistant" {
        output.push_str(assistant);
    }
    bounds::finish(output)
}

fn framing<'text>(
    style: Style,
    role: &str,
    position: (usize, Option<&str>),
    output: &mut String,
) -> Option<(&'text str, &'text str)> {
    let (index, previous) = position;
    let (human, assistant) = match style {
        Style::Claude | Style::Claude21 => ("\n\nHuman: ", "\n\nAssistant: "),
        Style::Titan => ("\n\nUser: ", "\n\nBot: "),
    };
    Some(match (style, role) {
        (Style::Claude21, "system") => ("", ""),
        (Style::Claude21, "assistant") => {
            if previous == Some("system") {
                output.push_str(human);
            }
            (assistant, "")
        }
        (_, "user") => (human, ""),
        (Style::Claude | Style::Titan, "system") => {
            output.push_str(human);
            ("<admin>", "</admin>")
        }
        (Style::Claude21, _) => {
            return None;
        }
        (Style::Claude | Style::Titan, _) => {
            if index == 0 && role == "assistant" {
                output.push_str(human);
            }
            (assistant, "")
        }
    })
}
