use std::string::FromUtf8Error;

use betterlite_http::proxy::limits::ReadError;
use reqwest::header::InvalidHeaderValue;
use thiserror::Error;

use crate::model_armor::decisions::ModelArmorAPIError;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{detail}")]
pub struct ArmorTokenError {
    pub detail: String,
}

#[derive(Debug, Error)]
pub enum TransportError<E> {
    #[error("Model Armor needs a template id")]
    MissingTemplate,
    #[error("Model Armor needs text content or file bytes")]
    NoContent,
    #[error("the Model Armor access token could not be resolved")]
    Token(#[source] E),
    #[error("the Model Armor request failed")]
    Request(#[source] reqwest::Error),
    #[error("the Model Armor request timed out")]
    Timeout(#[source] reqwest::Error),
    #[error("the Model Armor Authorization header is invalid")]
    Header(#[source] InvalidHeaderValue),
    #[error("the Model Armor response body could not be read")]
    Body(#[source] ReadError),
    #[error("the Model Armor response is not UTF-8")]
    ResponseText(#[source] FromUtf8Error),
    #[error("the Model Armor response is not valid JSON")]
    ResponseJson(#[source] serde_json::Error),
}

#[derive(Debug, Error)]
pub enum ArmorCallError<E> {
    #[error(transparent)]
    Api(#[from] ModelArmorAPIError),
    #[error(transparent)]
    Transport(#[from] TransportError<E>),
}
