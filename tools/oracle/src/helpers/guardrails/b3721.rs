use crate::helpers::{Helper, HelperError, HelperResult};
use betterlite_guardrails::bedrock_masking::{
    apply_masking_to_messages, apply_masking_to_response, extract_masked_texts,
};
use betterlite_guardrails::bedrock_message_selection::{
    collect_grounding_blocks, create_input_content_request, merge_masked_texts,
    select_messages_for_apply_guardrail,
};
use serde::Deserialize;
use serde_json::{Value, json, to_value};

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    step: Step,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Step {
    Input {
        messages: Vec<Value>,
    },
    Grounding {
        messages: Vec<Value>,
        contextual_fallback: bool,
    },
    Selection {
        texts: Vec<String>,
        messages: Vec<Value>,
        latest_only: bool,
    },
    Merge {
        masked: Vec<String>,
        texts: Vec<String>,
        scanned_slice: Option<(usize, usize)>,
        scanned_role_subset: bool,
    },
    Extract {
        response: Option<Value>,
    },
    MaskMessages {
        messages: Vec<Value>,
        masked_texts: Vec<String>,
    },
    MaskResponse {
        response: Value,
        masked_texts: Vec<String>,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let result = match &input.step {
            Step::Input { messages } => to_value(create_input_content_request(Some(messages))),
            Step::Grounding {
                messages,
                contextual_fallback,
            } => {
                let blocks = collect_grounding_blocks(Some(messages), *contextual_fallback);
                to_value(
                    blocks
                        .into_iter()
                        .map(|block| json!({"text": block.text, "qualifier": block.qualifier}))
                        .collect::<Vec<_>>(),
                )
            }
            Step::Selection {
                texts,
                messages,
                latest_only,
            } => {
                let selection =
                    select_messages_for_apply_guardrail(texts, Some(messages), *latest_only);
                to_value(
                    json!({"filtered_messages": selection.filtered_messages, "scanned_slice": selection.scanned_slice, "scanned_role_subset": selection.scanned_role_subset, "skip_scan": selection.skip_scan}),
                )
            }
            Step::Merge {
                masked,
                texts,
                scanned_slice,
                scanned_role_subset,
            } => to_value(merge_masked_texts(
                masked,
                texts,
                *scanned_slice,
                *scanned_role_subset,
            )),
            Step::Extract { response } => to_value(extract_masked_texts(response.as_ref())),
            Step::MaskMessages {
                messages,
                masked_texts,
            } => to_value(apply_masking_to_messages(messages, masked_texts)),
            Step::MaskResponse {
                response,
                masked_texts,
            } => {
                let mut response = response.clone();
                apply_masking_to_response(&mut response, masked_texts);
                to_value(response)
            }
        };
        result.map_err(|error| {
            Box::new(HelperError {
                class: "SerializationError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
    }
}
