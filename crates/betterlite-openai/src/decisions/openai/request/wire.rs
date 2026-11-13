use betterlite_models::decisions::openai::{ChoiceValue, MessageKind, Role};
use serde::Serialize;
use serde_with::skip_serializing_none;

#[skip_serializing_none]
#[derive(Debug, Serialize)]
pub struct RequestWire {
    pub model: String,
    pub input: InputWire,
    pub questions: Vec<QuestionWire>,
    pub safety_identifier: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum InputWire {
    Text(String),
    Messages(Vec<MessageWire>),
}

#[skip_serializing_none]
#[derive(Debug, Serialize)]
pub struct MessageWire {
    pub role: Role,
    #[serde(rename = "type")]
    pub kind: MessageKind,
    pub content: ContentWire,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum ContentWire {
    Text(String),
    Parts(Vec<PartWire>),
}

#[skip_serializing_none]
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PartWire {
    InputText {
        text: String,
    },
    InputImage {
        image_url: String,
        detail: Option<String>,
    },
}

#[skip_serializing_none]
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum QuestionWire {
    Predicate {
        name: Option<String>,
        instructions: String,
    },
    Choice {
        name: Option<String>,
        instructions: String,
        choices: Vec<ChoiceWire>,
    },
    Score {
        name: Option<String>,
        instructions: String,
        levels: Vec<LevelWire>,
    },
}

#[skip_serializing_none]
#[derive(Debug, Serialize)]
pub struct ChoiceWire {
    pub value: ChoiceValue,
    pub description: Option<String>,
}

#[skip_serializing_none]
#[derive(Debug, Serialize)]
pub struct LevelWire {
    pub label: String,
    pub description: Option<String>,
}
