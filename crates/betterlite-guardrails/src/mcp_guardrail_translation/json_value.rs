use serde::{Deserialize, Serialize};
use serde_json::Number;
use std::collections::BTreeMap;

const PRIVATE_NUMBER_KEY: &str = "$serde_json::private::Number";
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
#[serde(from = "WireValue")]
pub enum McpValue {
    Object(BTreeMap<String, Self>),
    Null,
    Boolean(bool),
    Number(Number),
    Text(String),
    Array(Vec<Self>),
}
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum WireValue {
    PrivateNumber(PrivateNumber),
    Object(BTreeMap<String, Self>),
    Null,
    Boolean(bool),
    Number(Number),
    Text(String),
    Array(Vec<Self>),
}
#[derive(Debug, Deserialize)]
struct PrivateNumber {
    #[serde(rename = "$serde_json::private::Number")]
    number: String,
    #[serde(flatten)]
    extra: BTreeMap<String, WireValue>,
}
impl From<WireValue> for McpValue {
    fn from(wire: WireValue) -> Self {
        match wire {
            WireValue::PrivateNumber(private) => private_number(private),
            WireValue::Object(entries) => Self::Object(
                entries
                    .into_iter()
                    .map(|(key, child)| (key, Self::from(child)))
                    .collect(),
            ),
            WireValue::Null => Self::Null,
            WireValue::Boolean(flag) => Self::Boolean(flag),
            WireValue::Number(number) => Self::Number(number),
            WireValue::Text(text) => Self::Text(text),
            WireValue::Array(children) => {
                Self::Array(children.into_iter().map(Self::from).collect())
            }
        }
    }
}
fn private_number(private: PrivateNumber) -> McpValue {
    let PrivateNumber {
        number: text,
        extra,
    } = private;
    if extra.is_empty()
        && let Ok(number) = text.parse()
    {
        return McpValue::Number(number);
    }
    let mut entries = extra;
    entries.insert(PRIVATE_NUMBER_KEY.to_owned(), WireValue::Text(text));
    McpValue::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key, McpValue::from(value)))
            .collect(),
    )
}

mod tests;
