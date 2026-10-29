use super::RenderError;
use crate::builtin::{Calls, Content, Message};
use indexmap::IndexMap;
use minijinja::value::{Serde, Value};
use minijinja::{Environment, context};
use std::collections::BTreeMap;

const ALTERNATION_MARKER: &str = "Conversation roles must alternate user/assistant";

fn probe_supports_system(template: &minijinja::Template<'_, '_>) -> bool {
    let probe = [Message {
        role: "system".to_owned(),
        content: Content::Text("test".to_owned()),
        tool_calls: Calls::Absent,
        extra: IndexMap::new(),
    }];
    let ctx = context! {
        bos_token => "<bos>",
        eos_token => "<eos>",
        messages => Value::from(Serde(&probe[..])),
    };
    template.render(ctx).is_ok()
}

fn pad_for_alternation(reformatted: &[Message]) -> Option<Vec<Message>> {
    let (first, _) = reformatted.split_first()?;
    let mut padded = vec![first.clone()];
    for pair in reformatted.windows(2) {
        let [previous, current] = pair else { continue };
        if previous.role == current.role {
            let opposite = if previous.role == "user" {
                "assistant"
            } else {
                "user"
            };
            padded.push(Message {
                role: opposite.to_owned(),
                content: Content::Text(String::new()),
                tool_calls: Calls::Absent,
                extra: IndexMap::new(),
            });
        }
        padded.push(current.clone());
    }
    Some(padded)
}

fn render_messages(
    template: &minijinja::Template<'_, '_>,
    bos_token: &str,
    eos_token: &str,
    messages: &[Message],
    add_generation_prompt: Option<bool>,
) -> Result<String, minijinja::Error> {
    let mut ctx = BTreeMap::from([
        ("bos_token", Value::from(bos_token)),
        ("eos_token", Value::from(eos_token)),
        ("messages", Value::from(Serde(messages))),
    ]);
    if let Some(flag) = add_generation_prompt {
        ctx.insert("add_generation_prompt", Value::from(flag));
    }
    template.render(ctx)
}

fn render_alternation_retry(
    template: &minijinja::Template<'_, '_>,
    bos_token: &str,
    eos_token: &str,
    reformatted: &[Message],
) -> Result<String, RenderError> {
    let Some(padded) = pad_for_alternation(reformatted) else {
        return Err(RenderError::Render {
            source: minijinja::Error::new(
                minijinja::ErrorKind::InvalidOperation,
                "reformatted messages are empty",
            ),
        });
    };
    render_messages(template, bos_token, eos_token, &padded, None)
        .map_err(|source| RenderError::Render { source })
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
        return render_messages(&template, bos_token, eos_token, messages, Some(true))
            .map_err(|source| RenderError::Render { source });
    }

    let reformatted: Vec<Message> = messages
        .iter()
        .map(|message| {
            if message.role == "system" {
                Message {
                    role: "user".to_owned(),
                    content: message.content.clone(),
                    tool_calls: Calls::Absent,
                    extra: IndexMap::new(),
                }
            } else {
                message.clone()
            }
        })
        .collect();
    match render_messages(&template, bos_token, eos_token, &reformatted, Some(true)) {
        Ok(rendered) => Ok(rendered),
        Err(error) if error.to_string().contains(ALTERNATION_MARKER) => {
            render_alternation_retry(&template, bos_token, eos_token, &reformatted)
        }
        Err(_) => Ok(String::new()),
    }
}

mod tests;
