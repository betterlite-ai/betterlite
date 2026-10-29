use indexmap::IndexMap;

use super::error::ImageModelsError;

/// A caller-supplied Bedrock image parameter value.
///
/// Bedrock's request bodies accept arbitrary provider-native keys passed through
/// unchanged, so this stays a small, explicitly named alternative to `serde_json::Value`
/// (see `crate::request_policy::RequestValue` for the same pattern applied to
/// header/identity merging).
#[derive(Debug, Clone, PartialEq)]
pub enum Field {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
    List(Vec<Self>),
    Mapping(Params),
}

pub type Params = IndexMap<String, Field>;

impl Field {
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text),
            Self::Null
            | Self::Bool(_)
            | Self::Int(_)
            | Self::Float(_)
            | Self::List(_)
            | Self::Mapping(_) => None,
        }
    }

    /// Python truthiness: `None`/empty string/empty collection/zero/`False` are falsy.
    #[must_use]
    pub fn truthy(&self) -> bool {
        match self {
            Self::Null => false,
            Self::Bool(flag) => *flag,
            Self::Int(value) => *value != 0,
            Self::Float(value) => *value != 0.0,
            Self::Text(text) => !text.is_empty(),
            Self::List(items) => !items.is_empty(),
            Self::Mapping(fields) => !fields.is_empty(),
        }
    }

    /// Python's `str()` for the values that can reach an f-string in this module.
    #[must_use]
    pub fn python_str(&self) -> String {
        match self {
            Self::Null => "None".to_owned(),
            Self::Bool(true) => "True".to_owned(),
            Self::Bool(false) => "False".to_owned(),
            Self::Text(text) => text.clone(),
            Self::Int(value) => value.to_string(),
            Self::Float(value) => value.to_string(),
            Self::List(_) | Self::Mapping(_) => "object".to_owned(),
        }
    }

    #[must_use]
    pub const fn python_type_name(&self) -> &'static str {
        match self {
            Self::Null => "NoneType",
            Self::Bool(_) => "bool",
            Self::Int(_) => "int",
            Self::Float(_) => "float",
            Self::Text(_) => "str",
            Self::List(_) => "list",
            Self::Mapping(_) => "dict",
        }
    }
}

impl From<&str> for Field {
    fn from(value: &str) -> Self {
        Self::Text(value.to_owned())
    }
}

impl From<String> for Field {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<i64> for Field {
    fn from(value: i64) -> Self {
        Self::Int(value)
    }
}

impl From<bool> for Field {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<Params> for Field {
    fn from(value: Params) -> Self {
        Self::Mapping(value)
    }
}

/// Mirrors `dict.pop(key, {})` followed by relying on the result being a mapping: a
/// present, non-mapping value fails at this step rather than being silently coerced.
pub(super) fn pop_mapping(
    params: &mut Params,
    key: &'static str,
) -> Result<Params, ImageModelsError> {
    match params.shift_remove(key) {
        None => Ok(Params::new()),
        Some(Field::Mapping(mapping)) => Ok(mapping),
        Some(_) => Err(ImageModelsError::NotAMapping(key)),
    }
}
