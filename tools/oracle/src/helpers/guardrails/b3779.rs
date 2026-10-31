use std::collections::BTreeMap;

use betterlite_guardrails::lakera_pii::masking as lakera_masking;
use betterlite_guardrails::lakera_pii::wire::{LakeraResponse, Message as LakeraMessage};
use betterlite_guardrails::lasso::wire::MessageType;
use betterlite_guardrails::lasso_masking::wire::{
    LassoMessage, Message as OpenAiMessage, ModelResponseShape, ResponseChoice, ToolCall,
};
use betterlite_guardrails::lasso_masking::{apply_response, expand, lifecycle, map_back};
use serde_json::{Value, json};

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
        #[serde(default)]
        input: Option<String>,
        #[serde(default)]
        mask: bool,
    },
    #[serde(rename = "lifecycle_post_call_hook")]
    PostCall {
        messages: Vec<OpenAiMessage>,
        #[serde(default)]
        input: Option<String>,
        #[serde(default)]
        mask: bool,
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
            Input::Expand { messages } => json!(expand::expand_for_classification(messages)),
            Input::PreCall {
                messages,
                input: input_text,
                mask,
            } => lifecycle_result(messages, input_text.as_deref(), *mask, MessageType::Prompt),
            Input::PostCall {
                messages,
                input: input_text,
                mask,
            } => lifecycle_result(
                messages,
                input_text.as_deref(),
                *mask,
                MessageType::Completion,
            ),
            Input::MapMaskedMessagesBack { original, masked } => {
                json!(map_back::map_masked_messages_back(original, masked))
            }
            Input::ApplyMaskingToModelResponse { response, masked } => {
                apply_response_result(response, masked)
            }
            Input::MaskPiiInMessages { messages, response } => mask_pii_result(messages, response),
            Input::IsOnlyPiiViolation { response } => {
                json!(lakera_masking::is_only_pii_violation(Some(response)))
            }
            Input::Blocked { response } => blocked_result(response),
        };
        Ok(result)
    }
}

fn apply_response_result(response: &ModelResponseShape, masked: &[LassoMessage]) -> Value {
    let updated = apply_response::mask_model_response(response, masked);
    let choices: Vec<_> = updated.choices.iter().map(choice_projection).collect();
    json!(choices)
}

fn mask_pii_result(messages: &[LakeraMessage], response: &LakeraResponse) -> Value {
    let mut counts = BTreeMap::new();
    let masked = lakera_masking::mask_pii_in_messages(messages, Some(response), &mut counts);
    json!({"messages": masked, "counts": counts})
}

fn blocked_result(response: &LakeraResponse) -> Value {
    let error = lakera_masking::blocked_error(Some(response));
    json!({"status_code": error.status().as_u16(), "detail": {"error": error.to_string(), "lakera_guardrail_response": error.response()}})
}

fn lifecycle_result(
    messages: &[OpenAiMessage],
    input: Option<&str>,
    mask: bool,
    message_type: MessageType,
) -> Value {
    let result = lifecycle::run(messages, input, mask, message_type, None);
    json!(result.inspected)
}

fn choice_projection(choice: &ResponseChoice) -> Value {
    let message = choice.message.as_ref();
    let tool_calls: Vec<_> = message
        .and_then(|response_message| response_message.tool_calls.as_ref())
        .into_iter()
        .flatten()
        .map(|call: &ToolCall| {
            json!({
                "id": call.id,
                "arguments": call.function.as_ref().and_then(|function| function.arguments.clone()),
            })
        })
        .collect();
    json!({
        "content": message.and_then(|response_message| response_message.content.clone()),
        "tool_calls": tool_calls,
    })
}
