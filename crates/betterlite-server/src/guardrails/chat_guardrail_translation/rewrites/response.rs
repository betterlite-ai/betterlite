use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_message_scope::Object;
use serde::{Deserialize, Serialize};
use serde_with::{DefaultOnError, DefaultOnNull, serde_as};

// The fields the guardrail pass reads or rewrites; a null index counts as choice 0.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct ChatResponse {
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub choices: Vec<ResponseChoice>,
}

#[serde_as]
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct ResponseChoice {
    #[serde_as(as = "DefaultOnNull")]
    #[serde(default)]
    pub index: i64,
    #[serde(default)]
    pub message: ResponseMessage,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct ResponseMessage {
    #[serde(default)]
    pub content: Option<MessageContent>,
    #[serde(default)]
    pub tool_calls: Option<Vec<ResponseToolCall>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

#[serde_as]
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct ContentPart {
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde_as(as = "DefaultOnError")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_url: Option<ImageUrl>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct ImageUrl {
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct ResponseToolCall {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub function: Option<ResponseFunction>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct ResponseFunction {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<String>,
}

fn optional_text(value: Option<&String>) -> GuardrailValue {
    value.map_or(GuardrailValue::Null, |text| {
        GuardrailValue::Text(text.clone())
    })
}

impl ResponseToolCall {
    // The dict LiteLLM builds from a tool call object; a call without a function cannot be converted.
    #[must_use]
    pub fn to_object(&self) -> Option<Object> {
        let function = self.function.as_ref()?;
        let mut function_object = Object::new();
        function_object.insert("name".to_owned(), optional_text(function.name.as_ref()));
        function_object.insert(
            "arguments".to_owned(),
            optional_text(function.arguments.as_ref()),
        );
        let mut object = Object::new();
        object.insert("id".to_owned(), optional_text(self.id.as_ref()));
        object.insert(
            "type".to_owned(),
            GuardrailValue::Text(self.kind.clone().unwrap_or_else(|| "function".to_owned())),
        );
        object.insert(
            "function".to_owned(),
            GuardrailValue::Object(function_object),
        );
        Some(object)
    }
}
