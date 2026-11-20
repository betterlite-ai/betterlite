use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::RerankDocument;
use indexmap::IndexMap;
use secrecy::SecretString;
use std::collections::BTreeMap;
use thiserror::Error;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReturnDocuments {
    Default,
    Null,
    Value(bool),
}
impl ReturnDocuments {
    #[must_use]
    pub const fn mapped(self) -> Option<bool> {
        match self {
            Self::Default => Some(true),
            Self::Null => None,
            Self::Value(value) => Some(value),
        }
    }
}
#[derive(Debug, Clone)]
pub struct RerankCall {
    pub model: String,
    pub query: String,
    pub documents: Vec<RerankDocument>,
    pub custom_llm_provider: Option<String>,
    pub top_n: Option<i64>,
    pub rank_fields: Option<Vec<String>>,
    pub return_documents: ReturnDocuments,
    pub max_chunks_per_doc: Option<i64>,
    pub max_tokens_per_doc: Option<i64>,
    pub kwargs: IndexMap<String, Json>,
}
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CallError {
    #[error("{field} must be a string")]
    FieldText { field: &'static str },
    #[error("{field} must be an object of string values")]
    FieldHeaders { field: &'static str },
    #[error("{field} must be a boolean")]
    FieldFlag { field: &'static str },
    #[error("{field} must be a number or a string")]
    FieldTimeout { field: &'static str },
    #[error("could not convert string to float: '{value}'")]
    Timeout { value: String },
}
#[derive(Debug, Clone, PartialEq)]
pub enum TimeoutArg {
    Number(f64),
    Text(String),
}
impl TimeoutArg {
    pub fn seconds(&self) -> Result<f64, CallError> {
        match self {
            Self::Number(value) => Ok(*value),
            Self::Text(text) => text.parse::<f64>().map_err(|_| CallError::Timeout {
                value: text.clone(),
            }),
        }
    }
}
#[derive(Debug, Clone, Default)]
pub struct GenericParams {
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub timeout: Option<TimeoutArg>,
    pub drop_params: Option<bool>,
    pub user: Option<String>,
    pub headers: BTreeMap<String, String>,
    pub extra_headers: Option<BTreeMap<String, String>>,
    pub instruction: Option<String>,
}
impl GenericParams {
    pub fn read(kwargs: &IndexMap<String, Json>) -> Result<Self, CallError> {
        let mut params = Self::default();
        for (name, value) in kwargs {
            match name.as_str() {
                "api_key" => params.api_key = string(value, "api_key")?.map(SecretString::from),
                "api_base" => params.api_base = string(value, "api_base")?,
                "timeout" => {
                    params.timeout = match value {
                        Json::Null => None,
                        Json::Number(number) => Some(TimeoutArg::Number(
                            number
                                .as_f64()
                                .ok_or(CallError::FieldTimeout { field: "timeout" })?,
                        )),
                        Json::Text(text) => Some(TimeoutArg::Text(text.clone())),
                        Json::Bool(_) | Json::List(_) | Json::Object(_) => {
                            return Err(CallError::FieldTimeout { field: "timeout" });
                        }
                    };
                }
                "drop_params" => {
                    params.drop_params = match value {
                        Json::Null => None,
                        Json::Bool(flag) => Some(*flag),
                        Json::Text(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                            return Err(CallError::FieldFlag {
                                field: "drop_params",
                            });
                        }
                    };
                }
                "user" => params.user = string(value, "user")?,
                "headers" => params.headers = header_map(value, "headers")?.unwrap_or_default(),
                "extra_headers" => params.extra_headers = header_map(value, "extra_headers")?,
                "instruction" => params.instruction = string(value, "instruction")?,
                _ => {}
            }
        }
        Ok(params)
    }
}
fn string(value: &Json, field: &'static str) -> Result<Option<String>, CallError> {
    match value {
        Json::Null => Ok(None),
        Json::Text(text) => Ok(Some(text.clone())),
        Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            Err(CallError::FieldText { field })
        }
    }
}
fn header_map(
    value: &Json,
    field: &'static str,
) -> Result<Option<BTreeMap<String, String>>, CallError> {
    if matches!(value, Json::Null) {
        return Ok(None);
    }
    let Json::Object(fields) = value else {
        return Err(CallError::FieldHeaders { field });
    };
    let mut mapped = BTreeMap::new();
    for (name, value) in fields {
        let Json::Text(text) = value else {
            return Err(CallError::FieldHeaders { field });
        };
        mapped.insert(name.clone(), text.clone());
    }
    Ok(Some(mapped))
}
