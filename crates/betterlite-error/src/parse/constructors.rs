#[derive(Clone, Copy)]
pub(super) enum Constructor {
    Response,
    BodyResponse,
    Plain,
    MissingArguments,
    NoSignature,
}

pub(super) fn constructor(name: &str) -> Option<Constructor> {
    match name {
        "AuthenticationError"
        | "BadGatewayError"
        | "ContextWindowExceededError"
        | "NotFoundError"
        | "PermissionDeniedError"
        | "ServiceUnavailableError"
        | "UnprocessableEntityError"
        | "UnsupportedParamsError" => Some(Constructor::Response),
        "BadRequestError"
        | "ContentPolicyViolationError"
        | "ImageFetchError"
        | "InternalServerError"
        | "RateLimitError" => Some(Constructor::BodyResponse),
        "APIConnectionError" | "APIResponseValidationError" | "InvalidRequestError" => {
            Some(Constructor::Plain)
        }
        "APIError"
        | "BudgetExceededError"
        | "ErrorEventError"
        | "JSONSchemaValidationError"
        | "OpenAIError"
        | "RouterRateLimitError"
        | "VectorStoreSearchError" => Some(Constructor::MissingArguments),
        "ModelNotMappedError" | "ZoneInfoNotFoundError" => Some(Constructor::NoSignature),
        _ => None,
    }
}
