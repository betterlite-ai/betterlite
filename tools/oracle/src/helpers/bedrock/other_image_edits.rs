use std::io;
use std::str;

use betterlite_bedrock::image_edits::error::ImageEditError;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_models::catalog::CatalogError;
use betterlite_redact::error::RedactionError;
use serde_json::Value;
use thiserror::Error;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod driver;
#[path = "other_image_edits/fixtures/support.rs"]
mod fixtures;
mod input;
mod tests;

use driver::observe;
use input::Input;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut results = Vec::with_capacity(input.probes.len());
        for probe in &input.probes {
            results.push(observe(probe, input.function, &input.model_cost)?);
        }
        Ok::<Value, Failure>(Value::Array(results)).map_err(Box::<HelperError>::from)
    }
}

#[derive(Debug, Error)]
pub(super) enum Failure {
    #[error(transparent)]
    ImageEdit(#[from] ImageEditError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Logging(#[from] LoggingStateError),
    #[error(transparent)]
    Redact(#[from] RedactionError),
    #[error(transparent)]
    Catalog(#[from] CatalogError),
    #[error("the probe body is not UTF-8")]
    Body(#[from] str::Utf8Error),
    #[error("the prepared value is not an object")]
    Shape,
    #[error("the async runtime could not start")]
    Runtime(#[from] io::Error),
}

impl Failure {
    pub(super) const fn status(&self) -> Option<u16> {
        match self {
            Self::ImageEdit(error) => error.status(),
            Self::Json(_)
            | Self::Logging(_)
            | Self::Redact(_)
            | Self::Catalog(_)
            | Self::Body(_)
            | Self::Shape
            | Self::Runtime(_) => None,
        }
    }
}

impl From<Failure> for Box<HelperError> {
    fn from(failure: Failure) -> Self {
        Self::new(HelperError {
            class: "ImageEditError".to_owned(),
            status: failure.status(),
            error_type: None,
            param: None,
            code: None,
            message: failure.to_string(),
        })
    }
}
