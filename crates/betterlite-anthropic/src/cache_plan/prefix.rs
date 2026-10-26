use serde::Serialize;
use serde_json::value::RawValue;

use crate::cache_plan::decode::{Decoded, Fields, Section};

#[derive(Serialize)]
struct PrefixBody<'plan> {
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<&'plan RawValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'plan RawValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_config: Option<&'plan RawValue>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<&'plan Fields>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    system: Vec<&'plan Fields>,
    messages: Vec<PrefixMessage<'plan>>,
}

#[derive(Serialize)]
struct PrefixMessage<'plan> {
    #[serde(skip)]
    index: i64,
    role: &'plan str,
    content: Vec<&'plan Fields>,
}

pub(super) fn prefix_body(
    decoded: &Decoded,
    last: usize,
) -> Result<Box<RawValue>, serde_json::Error> {
    let prefix = decoded.positions.get(..=last).unwrap_or_default();
    let section = |wanted: Section| {
        prefix
            .iter()
            .filter(|position| position.section == wanted)
            .map(|position| &position.fields)
            .collect()
    };
    let mut messages: Vec<PrefixMessage<'_>> = Vec::new();
    for position in prefix
        .iter()
        .filter(|position| position.section == Section::Messages)
    {
        match messages.last_mut() {
            Some(message) if message.index == position.message_index => {
                message.content.push(&position.fields);
            }
            Some(_) | None => messages.push(PrefixMessage {
                index: position.message_index,
                role: &position.role,
                content: vec![&position.fields],
            }),
        }
    }
    let body = PrefixBody {
        thinking: decoded.thinking.as_deref(),
        tool_choice: decoded.tool_choice.as_deref(),
        output_config: decoded.output_config.as_deref(),
        tools: section(Section::Tools),
        system: section(Section::System),
        messages,
    };
    serde_json::to_string(&body).and_then(RawValue::from_string)
}
