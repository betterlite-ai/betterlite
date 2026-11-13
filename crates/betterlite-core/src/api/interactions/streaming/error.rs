use std::error::Error;

use betterlite_http::error::HttpError;
use betterlite_openai::aleph_alpha::error::AlephAlphaError;
use thiserror::Error;

use crate::api::interactions::contract::config::BaseInteractionsAPIConfig;

#[derive(Debug, Error, PartialEq, Eq)]
#[error("the {field} field has the wrong type")]
pub struct FieldTypeError {
    pub field: &'static str,
}

#[derive(Debug, Error)]
pub enum InteractionSourceError {
    #[error("the interaction stream body could not be read")]
    Body {
        #[source]
        source: HttpError,
    },
    #[error(transparent)]
    Line(#[from] AlephAlphaError),
}

#[derive(Debug, Error)]
pub enum InteractionStreamError<SourceError, ConfigError>
where
    SourceError: Error + 'static,
    ConfigError: Error + 'static,
{
    #[error(transparent)]
    FieldType(#[from] FieldTypeError),
    #[error("the interaction stream line could not be read")]
    Source {
        #[source]
        source: SourceError,
    },
    #[error("the interaction stream chunk could not be transformed")]
    Transform {
        #[source]
        source: ConfigError,
    },
}

pub type AsyncStreamError<Response, Config> = InteractionStreamError<
    <Response as super::response::InteractionsAsyncResponse>::Error,
    <Config as BaseInteractionsAPIConfig>::Error,
>;

pub type SyncStreamError<Response, Config> = InteractionStreamError<
    <Response as super::response::InteractionsSyncResponse>::Error,
    <Config as BaseInteractionsAPIConfig>::Error,
>;
