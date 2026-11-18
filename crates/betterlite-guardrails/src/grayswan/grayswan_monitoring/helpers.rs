use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::generic_guardrail_contracts::GuardrailMessage;
use crate::guardrail_base::failure::GuardrailFailure;

use super::super::error::{GRAYSWAN_BLOCK_ERROR_MSG, GraySwanFailure, GraySwanGuardrailAPIError};

// LiteLLM's _as_plain_dict input: a plain wire row, or a typed model row that needs dumping
pub enum MessageRow<'row> {
    Plain(&'row GuardrailValue),
    Typed(&'row GuardrailMessage),
}

// LiteLLM's _as_plain_dict keeps a Mapping and dumps a BaseModel to its plain dict
#[must_use]
pub fn as_plain_dict(row: MessageRow<'_>) -> GuardrailValue {
    match row {
        MessageRow::Plain(value) => value.clone(),
        MessageRow::Typed(message) => {
            let mut fields = message.extra.clone();
            fields.insert(
                "role".to_owned(),
                GuardrailValue::Text(message.role.clone()),
            );
            GuardrailValue::Object(fields)
        }
    }
}

// The two exception kinds LiteLLM's _is_grayswan_exception looks at
pub enum GuardrailException<'error> {
    Api(&'error GraySwanGuardrailAPIError),
    Failure(&'error GuardrailFailure),
}

// LiteLLM's _is_grayswan_exception: a passthrough decision, or a block the API detail names
#[must_use]
pub fn is_grayswan_exception(exception: GuardrailException<'_>) -> bool {
    match exception {
        GuardrailException::Api(error) => {
            matches!(&error.cause, Some(GraySwanFailure::Blocked { .. }))
        }
        GuardrailException::Failure(failure) => match failure {
            GuardrailFailure::ModifyResponse(_) => true,
            GuardrailFailure::HttpObject { detail, .. } => matches!(
                detail.get("error"),
                Some(GuardrailValue::Text(error)) if error == GRAYSWAN_BLOCK_ERROR_MSG
            ),
            GuardrailFailure::GuardrailRaised { .. }
            | GuardrailFailure::BlockedPii { .. }
            | GuardrailFailure::SensitiveDataRoute(_)
            | GuardrailFailure::Http { .. }
            | GuardrailFailure::Other(_) => false,
        },
    }
}
