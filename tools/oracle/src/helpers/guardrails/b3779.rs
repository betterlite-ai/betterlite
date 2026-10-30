use std::collections::BTreeMap;

use betterlite_guardrails::{lakera_pii, lasso_masking};
use serde_json::{Value, json};

pub(crate) struct Op;

// The helper registration gate uses these operation labels to keep both primitive families discoverable.
const LASSO_MASKING_OPERATION: &str = "LassoMasking";
const LAKERA_PROCESSING_OPERATION: &str = "LakeraProcessing";

#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Input {
    #[serde(rename = "expand_messages_for_classification")]
    Expand {
        messages: Vec<Value>,
    },
    #[serde(rename = "lifecycle_pre_call_hook")]
    PreCall {
        messages: Vec<Value>,
    },
    #[serde(rename = "lifecycle_post_call_hook")]
    PostCall {
        messages: Vec<Value>,
    },
    MapMaskedMessagesBack {
        original: Vec<Value>,
        masked: Vec<Value>,
    },
    ApplyMaskingToModelResponse {
        response: Value,
        masked: Vec<Value>,
    },
    MaskPiiInMessages {
        messages: Vec<Value>,
        response: Value,
    },
    IsOnlyPiiViolation {
        response: Value,
    },
    #[serde(rename = "get_http_exception_for_blocked_guardrail")]
    Blocked {
        response: Value,
    },
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let _operation_labels = (LASSO_MASKING_OPERATION, LAKERA_PROCESSING_OPERATION);
        let result = match input {
            Input::Expand { messages } => {
                json!(lasso_masking::expand_for_classification(messages))
            }
            Input::PreCall { messages } | Input::PostCall { messages } => {
                json!(lasso_masking::expand_for_classification(messages))
            }
            Input::MapMaskedMessagesBack { original, masked } => {
                json!(lasso_masking::map_masked_messages_back(original, masked))
            }
            Input::ApplyMaskingToModelResponse { response, masked } => {
                let mut result = response.clone();
                lasso_masking::apply_model_response(&mut result, masked);
                result
            }
            Input::MaskPiiInMessages { messages, response } => {
                let mut counts = BTreeMap::new();
                json!({"messages":lakera_pii::mask_pii_in_messages(messages, Some(response), &mut counts),"counts":counts})
            }
            Input::IsOnlyPiiViolation { response } => {
                json!(lakera_pii::is_only_pii_violation(Some(response)))
            }
            Input::Blocked { response } => lakera_pii::blocked_exception(Some(response)),
        };
        Ok(result)
    }
}
