use serde::{Deserialize, Serialize};
use std::slice;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Function<Arguments> {
    pub name: String,
    pub arguments: Arguments,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Custom<Arguments> {
    pub name: String,
    pub input: Arguments,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ToolCall<Arguments> {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: Option<Function<Arguments>>,
    pub custom: Option<Custom<Arguments>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ParsedCall<Arguments> {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: Function<Arguments>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Message<Content, Call> {
    pub role: Option<String>,
    pub content: Option<Content>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<Call>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Input<Content, Arguments> {
    Text(String),
    Nested { message: Box<Self> },
    Message(Message<Content, ToolCall<Arguments>>),
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Messages<Content, Arguments> {
    Many(Vec<Input<Content, Arguments>>),
    One(Input<Content, Arguments>),
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Output<Content, Arguments> {
    Text(String),
    Message(Message<Content, ParsedCall<Arguments>>),
    Many(Vec<Self>),
}

pub fn parse_tool_calls<Arguments>(calls: &[ToolCall<Arguments>]) -> Vec<ParsedCall<Arguments>>
where
    Arguments: Clone,
{
    calls
        .iter()
        .filter_map(|call| {
            let function = match &call.custom {
                Some(custom) => Function {
                    name: custom.name.clone(),
                    arguments: custom.input.clone(),
                },
                None => call.function.clone()?,
            };
            Some(ParsedCall {
                id: call.id.clone(),
                kind: call.kind.clone(),
                function,
            })
        })
        .collect()
}

pub fn parse_messages<Content, Arguments>(
    input: &Messages<Content, Arguments>,
) -> Output<Content, Arguments>
where
    Content: Clone,
    Arguments: Clone,
{
    let items = match input {
        Messages::One(message) => slice::from_ref(message),
        Messages::Many(messages) => messages.as_slice(),
    };
    if let [message] = items {
        return Output::from(message);
    }
    Output::Many(items.iter().map(Output::from).collect())
}

impl<Content, Arguments> From<&Input<Content, Arguments>> for Output<Content, Arguments>
where
    Content: Clone,
    Arguments: Clone,
{
    fn from(input: &Input<Content, Arguments>) -> Self {
        match input {
            Input::Text(text) => Self::Text(text.clone()),
            Input::Nested { message } => Self::from(message.as_ref()),
            Input::Message(message) => Self::Message(Message {
                role: message.role.clone(),
                content: message.content.clone(),
                tool_calls: message
                    .tool_calls
                    .as_ref()
                    .filter(|calls| !calls.is_empty())
                    .map(|calls| parse_tool_calls(calls)),
            }),
        }
    }
}
