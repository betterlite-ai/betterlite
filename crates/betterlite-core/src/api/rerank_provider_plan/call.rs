use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::RerankDocument;
use indexmap::IndexMap;
use secrecy::SecretString;
use thiserror::Error;

/// LiteLLM's `return_documents: bool | None = True` argument: `Default` is the caller omitting it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReturnDocuments {
    Default,
    Null,
    Value(bool),
}

impl ReturnDocuments {
    /// The value the provider mapping receives: the default fills in `true`, and `Null` stays absent.
    #[must_use]
    pub const fn mapped(self) -> Option<bool> {
        match self {
            Self::Default => Some(true),
            Self::Null => None,
            Self::Value(value) => Some(value),
        }
    }
}

/// One call of LiteLLM's `rerank`: the declared arguments and the remaining `**kwargs` without `arerank`.
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
    /// `**kwargs` as the caller passed it; `arerank` is popped before any other use.
    pub kwargs: IndexMap<String, Json>,
}

/// The failure a malformed generic parameter raises before LiteLLM builds its parameter model.
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

/// A timeout argument: LiteLLM keeps a string until it converts it after the provider mapping.
#[derive(Debug, Clone, PartialEq)]
pub enum TimeoutArg {
    Number(f64),
    Text(String),
}

impl TimeoutArg {
    /// LiteLLM's `float(optional_params.timeout)` at main.py:180.
    pub fn seconds(&self) -> Result<f64, CallError> {
        match self {
            Self::Number(value) => Ok(*value),
            Self::Text(text) => text.parse::<f64>().map_err(|_| CallError::Timeout {
                value: text.clone(),
            }),
        }
    }
}

/// The generic parameters the dispatcher reads out of `kwargs` before it resolves the provider.
#[derive(Debug, Clone, Default, PartialEq)]
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
        Ok(Self {
            api_key: text(kwargs.get("api_key"), "api_key")?.map(SecretString::from),
            api_base: text(kwargs.get("api_base"), "api_base")?,
            timeout: timeout(kwargs.get("timeout"))?,
            drop_params: flag(kwargs.get("drop_params"))?,
            user: text(kwargs.get("user"), "user")?,
            headers: headers(kwargs.get("headers"), "headers")?.unwrap_or_default(),
            extra_headers: headers(kwargs.get("extra_headers"), "extra_headers")?,
            instruction: text(kwargs.get("instruction"), "instruction")?,
        })
    }
}

/// An absent or null value takes the field's default; a value of another type names the field.
pub(super) fn text(value: Option<&Json>, field: &'static str) -> Result<Option<String>, CallError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(CallError::FieldText { field })
        }
    }
}

fn flag(value: Option<&Json>) -> Result<Option<bool>, CallError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Bool(value)) => Ok(Some(*value)),
        Some(Json::Text(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(CallError::FieldFlag {
                field: "drop_params",
            })
        }
    }
}

fn timeout(value: Option<&Json>) -> Result<Option<TimeoutArg>, CallError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Number(number)) => number
            .as_f64()
            .map(|seconds| Some(TimeoutArg::Number(seconds)))
            .ok_or(CallError::FieldTimeout { field: "timeout" }),
        Some(Json::Text(text)) => Ok(Some(TimeoutArg::Text(text.clone()))),
        Some(Json::Bool(_) | Json::List(_) | Json::Object(_)) => {
            Err(CallError::FieldTimeout { field: "timeout" })
        }
    }
}

/// Header maps keep string values only, as a dict of non-strings does before httpx sees it.
fn headers(
    value: Option<&Json>,
    field: &'static str,
) -> Result<Option<BTreeMap<String, String>>, CallError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(fields)) => {
            let mut mapped = BTreeMap::new();
            for (name, value) in fields {
                let Json::Text(text) = value else {
                    return Err(CallError::FieldHeaders { field });
                };
                mapped.insert(name.clone(), text.clone());
            }
            Ok(Some(mapped))
        }
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(CallError::FieldHeaders { field })
        }
    }
}
