use std::convert::Infallible;
use std::error::Error;

use thiserror::Error;

use crate::api::stability::generation::reply::StabilityReplyError;

#[derive(Debug, Error)]
#[error("the output_cost_per_image value is not a number")]
pub struct CostValueError;

#[derive(Debug, Error)]
pub enum StabilityEditError<LookupError = Infallible>
where
    LookupError: Error + 'static,
{
    #[error("STABILITY_API_KEY is not set; set the environment variable or pass an api_key")]
    MissingApiKey,
    #[error(
        "the {parameter} parameter is not supported for model {model}; drop_params would drop it"
    )]
    UnsupportedParameter { parameter: String, model: String },
    #[error("the {field} value is not the type Stability AI accepts")]
    RequestField { field: &'static str },
    #[error("the {field} part needs at least one file")]
    MissingFile { field: &'static str },
    #[error(transparent)]
    CostValue(#[from] CostValueError),
    #[error(transparent)]
    Reply(#[from] StabilityReplyError),
    #[error(transparent)]
    ModelInfo(LookupError),
}
