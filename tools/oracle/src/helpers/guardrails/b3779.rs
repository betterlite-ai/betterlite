use std::collections::BTreeMap;

use betterlite_guardrails::lakera_pii::masking as lakera_masking;
use betterlite_guardrails::lakera_pii::wire::{LakeraResponse, Message as LakeraMessage};
use betterlite_guardrails::lasso_masking::wire::{
    LassoMessage, Message as OpenAiMessage, ModelResponseShape,
};
use betterlite_guardrails::lasso_masking::{apply_response, expand, map_back};
use serde_json::json;

// Bridges the oracle's b3779 operation to the LassoMasking and LakeraProcessing primitives.
pub(crate) struct Op;

#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    #[serde(rename = "expand_messages_for_classification")]
    Expand {
        messages: Vec<OpenAiMessage>,
    },
    #[serde(rename = "lifecycle_pre_call_hook")]
    PreCall {
        messages: Vec<OpenAiMessage>,
    },
    #[serde(rename = "lifecycle_post_call_hook")]
    PostCall {
        messages: Vec<OpenAiMessage>,
    },
    MapMaskedMessagesBack {
        original: Vec<OpenAiMessage>,
        masked: Vec<LassoMessage>,
    },
    ApplyMaskingToModelResponse {
        response: ModelResponseShape,
        masked: Vec<LassoMessage>,
    },
    MaskPiiInMessages {
        messages: Vec<LakeraMessage>,
        response: LakeraResponse,
    },
    IsOnlyPiiViolation {
        response: LakeraResponse,
    },
    #[serde(rename = "get_http_exception_for_blocked_guardrail")]
    Blocked {
        response: LakeraResponse,
    },
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let result = match input {
            Input::Expand { messages }
            | Input::PreCall { messages }
            | Input::PostCall { messages } => {
                json!(expand::expand_for_classification(messages))
            }
            Input::MapMaskedMessagesBack { original, masked } => {
                json!(map_back::map_masked_messages_back(original, masked))
            }
            Input::ApplyMaskingToModelResponse { response, masked } => {
                json!(apply_response::mask_model_response(response, masked))
            }
            Input::MaskPiiInMessages { messages, response } => {
                let mut counts = BTreeMap::new();
                let masked =
                    lakera_masking::mask_pii_in_messages(messages, Some(response), &mut counts);
                json!({"messages": masked, "counts": counts})
            }
            Input::IsOnlyPiiViolation { response } => {
                json!(lakera_masking::is_only_pii_violation(Some(response)))
            }
            Input::Blocked { response } => {
                let error = lakera_masking::blocked_error(Some(response));
                json!({
                    "status_code": error.status().as_u16(),
                    "detail": {
                        "error": error.to_string(),
                        "lakera_guardrail_response": error.response(),
                    },
                })
            }
        };
        Ok(result)
    }
}
