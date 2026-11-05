use indexmap::IndexMap;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::primitives::error::PrimitiveError;
use crate::primitives::json_value::JsonValue;
use crate::primitives::registry::Primitive;

#[derive(Deserialize)]
pub enum Never {}

// An empty map: no value can deserialize, so any argument is an error.
pub type NoArguments = IndexMap<String, Never>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockArguments {
    pub reason: String,
    pub detection_info: Option<IndexMap<String, JsonValue>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlagArguments {
    pub reason: String,
    pub metadata: Option<IndexMap<String, JsonValue>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModifyArguments {
    pub texts: Option<Vec<String>>,
    pub images: Option<Vec<JsonValue>>,
    pub tool_calls: Option<Vec<JsonValue>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatternArguments {
    pub text: String,
    pub pattern: String,
    pub flags: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplaceArguments {
    pub text: String,
    pub pattern: String,
    pub replacement: String,
    pub flags: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextArguments {
    pub text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StringifyArguments {
    pub obj: JsonValue,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchemaArguments {
    pub obj: JsonValue,
    pub schema: JsonValue,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UrlArguments {
    pub url: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpRequestArguments {
    pub url: String,
    pub method: Option<String>,
    pub headers: Option<IndexMap<String, String>>,
    pub body: Option<JsonValue>,
    pub timeout: Option<f64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpGetArguments {
    pub url: String,
    pub headers: Option<IndexMap<String, String>>,
    pub timeout: Option<f64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpPostArguments {
    pub url: String,
    pub body: Option<JsonValue>,
    pub headers: Option<IndexMap<String, String>>,
    pub timeout: Option<f64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LanguagesArguments {
    pub text: String,
    pub languages: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstringArguments {
    pub text: String,
    pub substring: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstringsArguments {
    pub text: String,
    pub substrings: Vec<String>,
}

// Positional values fill the signature's parameters in order, then named ones are added; a name used twice is an error.
pub fn bind<Arguments>(
    primitive: Primitive,
    positional: Vec<JsonValue>,
    named: IndexMap<String, JsonValue>,
) -> Result<Arguments, PrimitiveError>
where
    Arguments: DeserializeOwned,
{
    let (name, parameters) = primitive.signature();
    if positional.len() > parameters.len() {
        return Err(PrimitiveError::TooManyArguments {
            primitive: name,
            expected: parameters.len(),
            given: positional.len(),
        });
    }
    let mut bound: IndexMap<String, JsonValue> = IndexMap::new();
    for (parameter, value) in parameters.iter().zip(positional) {
        bound.insert((*parameter).to_owned(), value);
    }
    for (key, value) in named {
        if bound.contains_key(&key) {
            return Err(PrimitiveError::DuplicateArgument {
                primitive: name,
                name: key,
            });
        }
        bound.insert(key, value);
    }
    let encoded =
        serde_json::to_string(&bound).map_err(|source| PrimitiveError::Encode { source })?;
    serde_json::from_str(&encoded).map_err(|source| PrimitiveError::Arguments {
        primitive: name,
        source,
    })
}
