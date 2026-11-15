use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum StabilityGenerationError {
    #[error("STABILITY_API_KEY is not set; set the environment variable or pass an api_key")]
    MissingApiKey,
    #[error(
        "the {parameter} parameter is not supported for model {model}; drop_params would drop it"
    )]
    UnsupportedParameter { parameter: String, model: String },
    #[error("the {field} value is not the type Stability AI accepts")]
    RequestField { field: &'static str },
}
