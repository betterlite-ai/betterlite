use betterlite_error::error::ErrorClass;
use betterlite_error::error::details::{BudgetExceededDetails, SchemaValidationDetails};

// The text each LiteLLM exception's __init__ builds, so CallError's Display reports what str(error) reports
#[must_use]
pub(crate) fn error_message(class: &ErrorClass) -> String {
    match class {
        ErrorClass::Authentication(details) => details.message.clone(),
        ErrorClass::NotFound(details) => details.message.clone(),
        ErrorClass::PermissionDenied(details) => details.message.clone(),
        ErrorClass::UnprocessableEntity(details) => details.message.clone(),
        ErrorClass::BadRequest(details)
        | ErrorClass::ContextWindowExceeded(details)
        | ErrorClass::ContentPolicyViolation(details) => details.message.clone(),
        ErrorClass::GuardrailRaised(details) => details.message.clone(),
        ErrorClass::Timeout(details) => details.context.message.clone(),
        ErrorClass::RateLimit(details) => details.context.message.clone(),
        ErrorClass::BudgetExceeded(details) => budget_message(details),
        ErrorClass::ServiceUnavailable(context)
        | ErrorClass::BadGateway(context)
        | ErrorClass::InternalServer(context)
        | ErrorClass::ApiConnection(context)
        | ErrorClass::ApiResponseValidation(context)
        | ErrorClass::UnsupportedParams(context)
        | ErrorClass::Unclassified(context) => context.message.clone(),
        ErrorClass::Api(details) => details.context.message.clone(),
        ErrorClass::MidStreamFallback(details) => details.context.message.clone(),
        ErrorClass::JsonSchemaValidation(details) => schema_message(details),
    }
}

// BudgetExceededError.__init__ replaces an absent or empty message with the costs it was given
fn budget_message(details: &BudgetExceededDetails) -> String {
    details
        .message
        .as_deref()
        .filter(|message| !message.is_empty())
        .map_or_else(
            || {
                format!(
                    "Budget has been exceeded! Current cost: {}, Max budget: {}",
                    details.current_cost, details.max_budget
                )
            },
            str::to_owned,
        )
}

// JSONSchemaValidationError.__init__ builds its message from the identity and the response it validated
fn schema_message(details: &SchemaValidationDetails) -> String {
    format!(
        "litellm.JSONSchemaValidationError: model={}, returned an invalid response={}, for schema={}.\nAccess raw response with `e.raw_response`",
        details.model, details.raw_response, details.schema
    )
}
