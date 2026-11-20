use betterlite_error::error::ErrorClass;

use super::super::messages_fake_stream::MessagesFakeStreamError;
use super::super::messages_native_transform::error::TransformError;

// A callback's exception, as the callback raised it: the Python type name and the attributes @client reads
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{class}: {message}")]
pub struct RaisedException {
    pub class: String,
    pub message: String,
    pub status: Option<u16>,
    pub kind: Option<String>,
    pub param: Option<String>,
    pub code: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum MessagesExecuteError {
    // B571, B572 and the JSON round trip into their prompt types
    #[error(transparent)]
    Transform(#[from] TransformError),
    // B2435's fake SSE
    #[error(transparent)]
    FakeStream(#[from] MessagesFakeStreamError),
    // The source AttributeError when a null litellm_params has no provider to fall back on
    #[error("'{object_type}' object has no attribute 'get'")]
    MissingAttribute { object_type: String },
    // functools.partial refuses two values for one keyword before the handler runs
    #[error("functools.partial() got multiple values for keyword argument '{name}'")]
    DuplicateKeyword { name: String },
    // The advisor loop lands with B987; without it the request must never reach the provider
    #[error("the advisor_20260301 tool needs the advisor loop, which is not built yet")]
    AdvisorNotBuilt,
    // A hook map value the named parameter's declared type cannot hold
    #[error("the field {field} must be {expected}")]
    FieldType {
        field: &'static str,
        expected: &'static str,
    },
    // Errors from the cache hook, pre-request hooks, web-search and interceptors stay unclassified
    #[error(transparent)]
    Raised(Box<RaisedException>),
    // B011 already ran, so the composing caller renders this class instead of classifying it again
    #[error("the final execution's exception was classified by the dispatch boundary")]
    Classified(Box<ErrorClass>),
}
