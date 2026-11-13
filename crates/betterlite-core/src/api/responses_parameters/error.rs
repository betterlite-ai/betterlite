use betterlite_mcp::request_headers::raw_headers::DuplicateAuthorization;
use betterlite_paths::error::PathError;
use thiserror::Error;

// LiteLLM passes status_code=500, but UnsupportedParamsError.__init__ overwrites it with 400, so the SDK exception a caller sees carries 400.
pub const UNSUPPORTED_PARAMS_STATUS: u16 = 400;

#[derive(Debug, Error)]
pub enum ResponsesParametersError {
    #[error(
        "{provider} does not support parameters: {params}, for model {model}. To drop these, set `litellm.drop_params=True` or for proxy: `litellm_settings`: `drop_params: true`"
    )]
    UnsupportedParams {
        provider: String,
        model: String,
        params: String,
    },
    #[error("{field} must be an object")]
    NotAnObject { field: &'static str },
    #[error("{field} must be a list")]
    NotList { field: &'static str },
    #[error("{field} must be a string")]
    NotText { field: &'static str },
    #[error("{field} must be a string or a list of input items")]
    NotTextOrList { field: &'static str },
    #[error("the message source {index} does not name a client message")]
    MessageSource { index: usize },
    #[error("the original input items could not be split around the merged messages")]
    OriginalItems,
    #[error("the drop path {path} cannot be applied")]
    DropPath {
        path: String,
        #[source]
        source: PathError,
    },
    #[error("the parameters could not be read as JSON")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("multiple Authorization headers are not allowed")]
    DuplicateAuthorization {
        #[source]
        source: DuplicateAuthorization,
    },
    #[error("text_format must be an object")]
    TextFormatNotObject,
    #[error("text_format has no {field}")]
    MissingTextFormatField { field: &'static str },
}
