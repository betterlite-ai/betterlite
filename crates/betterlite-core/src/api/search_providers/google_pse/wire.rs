use std::fmt;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::form_urlencoded::Serializer;

#[derive(Debug, Error)]
pub enum WireError {
    #[error("{field} must be a string or an integer")]
    WrongType {
        field: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("{field} holds a list, which this provider cannot send")]
    ListValue { field: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SearchParam {
    Text(String),
    Number(i64),
    Many(Vec<Self>),
}

impl SearchParam {
    fn text(&self) -> Option<String> {
        match self {
            Self::Text(text) => Some(text.clone()),
            Self::Number(number) => Some(number.to_string()),
            Self::Many(_) => None,
        }
    }
}

pub(crate) fn debug_body(
    formatter: &mut fmt::Formatter<'_>,
    struct_name: &str,
    credential: &str,
    params: &IndexMap<String, SearchParam>,
) -> fmt::Result {
    let fields: Vec<(&str, String)> = params
        .iter()
        .map(|(name, param)| {
            let rendered = if name.as_str() == credential {
                String::from("[REDACTED]")
            } else {
                format!("{param:?}")
            };
            (name.as_str(), rendered)
        })
        .collect();
    formatter
        .debug_struct(struct_name)
        .field("params", &fields)
        .finish()
}

pub fn query_string(
    params: &IndexMap<String, SearchParam>,
    doseq: bool,
) -> Result<String, WireError> {
    let mut pairs: Vec<(String, String)> = Vec::new();
    for (name, param) in params {
        append_param(name, param, doseq, &mut pairs)?;
    }
    let mut encoder = Serializer::new(String::new());
    for (name, value) in &pairs {
        encoder.append_pair(name, value);
    }
    let quoted = encoder.finish();
    Ok(quoted.replace('*', "%2A").replace("%7E", "~"))
}

fn append_param(
    name: &str,
    param: &SearchParam,
    doseq: bool,
    pairs: &mut Vec<(String, String)>,
) -> Result<(), WireError> {
    let SearchParam::Many(values) = param else {
        let text = param.text().ok_or_else(|| WireError::ListValue {
            field: name.to_owned(),
        })?;
        pairs.push((name.to_owned(), text));
        return Ok(());
    };
    if !doseq {
        return Err(WireError::ListValue {
            field: name.to_owned(),
        });
    }
    for value in values {
        let text = value.text().ok_or_else(|| WireError::ListValue {
            field: name.to_owned(),
        })?;
        pairs.push((name.to_owned(), text));
    }
    Ok(())
}
