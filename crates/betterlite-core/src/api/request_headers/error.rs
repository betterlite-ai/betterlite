use std::num::{ParseFloatError, ParseIntError};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum RequestHeadersError {
    #[error("the {header} header must be a number")]
    Number {
        header: &'static str,
        #[source]
        source: ParseFloatError,
    },
    #[error("the {header} header must be an integer")]
    Integer {
        header: &'static str,
        #[source]
        source: ParseIntError,
    },
    #[error("the Cache-Control directive {directive} holds more than one '='")]
    CacheControl { directive: String },
    #[error("the key's user information could not be encoded as headers")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}
