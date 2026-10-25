use serde_json::value::RawValue;

use crate::cisco_conversation::Fields;

#[derive(Debug, Clone)]
pub enum Json {
    Null,
    Bool(bool),
    Number(String),
    Text(String),
    List(Vec<Box<RawValue>>),
    Object(Fields),
}

impl Json {
    #[must_use]
    pub fn parse(text: &str) -> Self {
        // the text comes from a RawValue, so parsing the kind its first byte names cannot fail
        match text.as_bytes().first() {
            Some(b'"') => serde_json::from_str(text).map_or(Self::Null, Self::Text),
            Some(b'[') => serde_json::from_str(text).map_or(Self::Null, Self::List),
            Some(b'{') => serde_json::from_str(text).map_or(Self::Null, Self::Object),
            Some(b't') => Self::Bool(true),
            Some(b'f') => Self::Bool(false),
            Some(b'n') | None => Self::Null,
            Some(_) => Self::Number(text.to_owned()),
        }
    }

    #[must_use]
    pub fn field(fields: &Fields, key: &str) -> Self {
        let text = fields.get(key).map_or("null", |raw| raw.get());
        Self::parse(text)
    }

    #[must_use]
    pub fn objects(&self) -> Vec<Fields> {
        match self {
            Self::List(items) => items.iter().filter_map(object_from_raw).collect(),
            Self::Null | Self::Bool(_) | Self::Number(_) | Self::Text(_) | Self::Object(_) => {
                Vec::new()
            }
        }
    }

    #[must_use]
    pub fn truthy(&self) -> bool {
        match self {
            Self::Null => false,
            Self::Bool(flag) => *flag,
            Self::Number(text) => number_is_nonzero(text),
            Self::Text(text) => !text.is_empty(),
            Self::List(items) => !items.is_empty(),
            Self::Object(fields) => !fields.is_empty(),
        }
    }

    // LiteLLM coerces with str(), so a string stays bare and everything else prints as its repr
    #[must_use]
    pub fn python_str(&self) -> String {
        match self {
            Self::Text(text) => text.clone(),
            Self::Null | Self::Bool(_) | Self::Number(_) | Self::List(_) | Self::Object(_) => {
                self.python_repr()
            }
        }
    }

    fn python_repr(&self) -> String {
        match self {
            Self::Null => "None".to_owned(),
            Self::Bool(true) => "True".to_owned(),
            Self::Bool(false) => "False".to_owned(),
            Self::Number(text) => text.clone(),
            Self::Text(text) => python_quote(text),
            Self::List(items) => {
                let reprs: Vec<String> = items
                    .iter()
                    .map(|item| Self::parse(item.get()).python_repr())
                    .collect();
                format!("[{}]", reprs.join(", "))
            }
            Self::Object(fields) => {
                let reprs: Vec<String> = fields
                    .iter()
                    .map(|(key, value)| {
                        format!(
                            "{}: {}",
                            python_quote(key),
                            Self::parse(value.get()).python_repr()
                        )
                    })
                    .collect();
                format!("{{{}}}", reprs.join(", "))
            }
        }
    }
}

fn object_from_raw(item: &Box<RawValue>) -> Option<Fields> {
    match Json::parse(item.get()) {
        Json::Object(fields) => Some(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

fn python_quote(text: &str) -> String {
    let mark = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut repr = String::from(mark);
    for character in text.chars() {
        match character {
            '\\' => repr.push_str("\\\\"),
            '\n' => repr.push_str("\\n"),
            '\r' => repr.push_str("\\r"),
            '\t' => repr.push_str("\\t"),
            _ if character == mark => repr.extend(['\\', mark]),
            _ if character.is_control() => {
                let escape = format!("\\x{:02x}", u32::from(character));
                repr.push_str(&escape);
            }
            _ => repr.push(character),
        }
    }
    repr.push(mark);
    repr
}

fn number_is_nonzero(text: &str) -> bool {
    let mantissa = text.split(['e', 'E']).next().unwrap_or_default();
    mantissa.chars().any(|digit| matches!(digit, '1'..='9'))
}
