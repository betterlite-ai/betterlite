use std::num::ParseIntError;

use thiserror::Error;

use crate::client::convert::ConvertError as PartConvertError;

#[derive(Debug, Error)]
pub(super) enum ConvertError {
    #[error("the A2A payload does not match the declared wire shape")]
    Decode(#[source] serde_json::Error),
    #[error("the converted A2A payload cannot be written as JSON")]
    Encode(#[source] serde_json::Error),
    #[error("the A2A part cannot be converted between protocol versions")]
    Part(#[from] PartConvertError),
    #[error("the converted A2A payload is not a JSON object")]
    NotAnObject,
    #[error("the A2A {label} carries no message, task or update")]
    MissingEvent { label: &'static str },
    #[error("the A2A agent card has no interface a 0.3 client can use")]
    NoLegacyInterface,
    #[error("the A2A integer field {field} is not a number")]
    Integer {
        field: &'static str,
        #[source]
        source: ParseIntError,
    },
    #[error("the A2A card is missing the required {field} member")]
    MissingCardField { field: &'static str },
}
