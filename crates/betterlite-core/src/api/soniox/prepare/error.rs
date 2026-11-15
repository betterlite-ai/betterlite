use thiserror::Error;

use crate::api::soniox::records::error::SonioxException;

#[derive(Debug, Error)]
pub enum PrepareError {
    #[error("the {field} value must be a number of seconds")]
    Interval { field: &'static str },
    #[error("the {field} value must be an integer number of attempts")]
    Attempts { field: &'static str },
    #[error("the {field} value must be a string")]
    Text { field: &'static str },
    #[error("the {field} value must be a string or a list of strings")]
    Cleanup { field: &'static str },
    #[error(transparent)]
    Auth(#[from] SonioxException),
}

#[derive(Debug, Error)]
pub enum ReplyError {
    #[error("the Soniox reply is {observed} bytes; the maximum is {limit}")]
    TooLarge { observed: usize, limit: usize },
    #[error("the Soniox reply has {observed} tokens; the maximum is {limit}")]
    TooManyTokens { observed: usize, limit: usize },
    #[error("the Soniox reply is not a JSON object")]
    NotObject,
    #[error("the Soniox reply has no {field} field")]
    Missing { field: &'static str },
    #[error("the {field} field of the Soniox reply must be {expected}")]
    Shape {
        field: &'static str,
        expected: &'static str,
    },
    #[error("the {field} field of the Soniox reply cannot be decoded")]
    Field {
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
}
