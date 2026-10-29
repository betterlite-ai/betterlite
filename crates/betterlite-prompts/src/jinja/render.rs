use super::RenderError;
use crate::builtin::{Calls, Content, Message};
use indexmap::IndexMap;
use minijinja::value::{Serde, Value};
use minijinja::{Environment, context};

const ALTERNATION_MARKER: &str = "Conversation roles must alternate user/assistant";

fn placeholder(role: &str, content: Content) -> Message {
    Message {
        role: role.to_owned(),
        content,
        tool_calls: Calls::Absent,
        extra: IndexMap::new(),
    }
}

fn probe_supports_system(template: &minijinja::Template<'_, '_>) -> bool {
    let probe = [placeholder("system", Content::Text("test".to_owned()))];
    let ctx = context! {
        bos_token => "<bos>",
        eos_token => "<eos>",
        messages => Value::from(Serde(&probe[..])),
    };
    template.render(ctx).is_ok()
}

fn reformat_for_user(messages: &[Message]) -> Vec<Message> {
    messages
        .iter()
        .map(|message| {
            if message.role == "system" {
                placeholder("user", message.content.clone())
            } else {
                message.clone()
            }
        })
        .collect()
}

fn pad_for_alternation(reformatted: &[Message]) -> Option<Vec<Message>> {
    let last = reformatted.last()?.clone();
    let mut padded = Vec::with_capacity(reformatted.len());
    for index in 0..reformatted.len() - 1 {
        padded.push(reformatted[index].clone());
        if reformatted[index].role == reformatted[index + 1].role {
            let opposite = if reformatted[index].role == "user" {
                "assistant"
            } else {
                "user"
            };
            padded.push(placeholder(opposite, Content::Text(String::new())));
        }
    }
    padded.push(last);
    Some(padded)
}

fn wrap(source: minijinja::Error) -> RenderError {
    RenderError::Render { source }
}

fn render_with_generation_prompt(
    template: &minijinja::Template<'_, '_>,
    bos_token: &str,
    eos_token: &str,
    messages: &[Message],
) -> Result<String, minijinja::Error> {
    let ctx = context! {
        bos_token,
        eos_token,
        messages => Value::from(Serde(messages)),
        add_generation_prompt => true,
    };
    template.render(ctx)
}

fn render_alternation_retry(
    template: &minijinja::Template<'_, '_>,
    bos_token: &str,
    eos_token: &str,
    reformatted: &[Message],
) -> Result<String, RenderError> {
    let Some(padded) = pad_for_alternation(reformatted) else {
        return Err(wrap(minijinja::Error::new(
            minijinja::ErrorKind::InvalidOperation,
            "reformatted messages are empty",
        )));
    };
    let ctx = context! {
        bos_token,
        eos_token,
        messages => Value::from(Serde(&padded)),
    };
    template.render(ctx).map_err(wrap)
}

pub(super) fn render_chat_template(
    env: &Environment<'_>,
    chat_template: &str,
    bos_token: &str,
    eos_token: &str,
    messages: &[Message],
) -> Result<String, RenderError> {
    let template = env
        .template_from_str(chat_template)
        .map_err(|source| RenderError::Compile { source })?;

    if probe_supports_system(&template) {
        return render_with_generation_prompt(&template, bos_token, eos_token, messages).map_err(wrap);
    }

    let reformatted = reformat_for_user(messages);
    match render_with_generation_prompt(&template, bos_token, eos_token, &reformatted) {
        Ok(rendered) => Ok(rendered),
        Err(error) if error.to_string().contains(ALTERNATION_MARKER) => {
            render_alternation_retry(&template, bos_token, eos_token, &reformatted)
        }
        Err(_) => Ok(String::new()),
    }
}

mod tests;
