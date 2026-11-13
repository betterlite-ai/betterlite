#[derive(Clone, Copy)]
pub(super) enum Constructor {
    Response,
    BodyResponse,
    Plain,
}

// the sixteen litellm.*Error names extract_exception can construct
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExceptionName {
    Authentication,
    BadGateway,
    ContextWindowExceeded,
    NotFound,
    PermissionDenied,
    ServiceUnavailable,
    UnprocessableEntity,
    UnsupportedParams,
    BadRequest,
    ContentPolicyViolation,
    ImageFetch,
    InternalServer,
    RateLimit,
    ApiConnection,
    ApiResponseValidation,
    InvalidRequest,
}

pub(super) enum Lookup {
    Buildable(ExceptionName),
    MissingArguments,
    NoSignature,
}

impl ExceptionName {
    pub(super) const fn constructor(self) -> Constructor {
        match self {
            Self::Authentication
            | Self::BadGateway
            | Self::ContextWindowExceeded
            | Self::NotFound
            | Self::PermissionDenied
            | Self::ServiceUnavailable
            | Self::UnprocessableEntity
            | Self::UnsupportedParams => Constructor::Response,
            Self::BadRequest
            | Self::ContentPolicyViolation
            | Self::ImageFetch
            | Self::InternalServer
            | Self::RateLimit => Constructor::BodyResponse,
            Self::ApiConnection | Self::ApiResponseValidation | Self::InvalidRequest => {
                Constructor::Plain
            }
        }
    }
}

pub(super) fn lookup(name: &str) -> Option<Lookup> {
    let buildable = match name {
        "AuthenticationError" => ExceptionName::Authentication,
        "BadGatewayError" => ExceptionName::BadGateway,
        "ContextWindowExceededError" => ExceptionName::ContextWindowExceeded,
        "NotFoundError" => ExceptionName::NotFound,
        "PermissionDeniedError" => ExceptionName::PermissionDenied,
        "ServiceUnavailableError" => ExceptionName::ServiceUnavailable,
        "UnprocessableEntityError" => ExceptionName::UnprocessableEntity,
        "UnsupportedParamsError" => ExceptionName::UnsupportedParams,
        "BadRequestError" => ExceptionName::BadRequest,
        "ContentPolicyViolationError" => ExceptionName::ContentPolicyViolation,
        "ImageFetchError" => ExceptionName::ImageFetch,
        "InternalServerError" => ExceptionName::InternalServer,
        "RateLimitError" => ExceptionName::RateLimit,
        "APIConnectionError" => ExceptionName::ApiConnection,
        "APIResponseValidationError" => ExceptionName::ApiResponseValidation,
        "InvalidRequestError" => ExceptionName::InvalidRequest,
        "APIError"
        | "BudgetExceededError"
        | "ErrorEventError"
        | "JSONSchemaValidationError"
        | "OpenAIError"
        | "RouterRateLimitError"
        | "VectorStoreSearchError" => return Some(Lookup::MissingArguments),
        "ModelNotMappedError" | "ZoneInfoNotFoundError" => return Some(Lookup::NoSignature),
        _ => return None,
    };
    Some(Lookup::Buildable(buildable))
}
