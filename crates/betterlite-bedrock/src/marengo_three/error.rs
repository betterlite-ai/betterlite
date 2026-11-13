use thiserror::Error;

use crate::bedrock_settings::{BedrockError, ErrorInput, HeaderError};

#[derive(Debug, Error)]
pub enum Marengo3Error {
    #[error("bedrock error ({status})")]
    Provider {
        status: u16,
        error: Box<BedrockError>,
    },
    #[error(transparent)]
    Header(#[from] HeaderError),
}

pub(super) fn bad_request(message: String) -> Marengo3Error {
    let input = ErrorInput {
        status_code: 400,
        message,
        request: None,
        body: None,
        status_code_is_synthesized: false,
        headers: None,
        response: None,
    };
    match BedrockError::try_from(input) {
        Ok(error) => Marengo3Error::Provider {
            status: 400,
            error: Box::new(error),
        },
        Err(source) => Marengo3Error::Header(source),
    }
}
