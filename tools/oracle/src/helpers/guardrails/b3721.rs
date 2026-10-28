use crate::helpers::{Helper, HelperError, HelperResult};
use betterlite_guardrails::bedrock_masking::{
    apply_to_messages, apply_to_response, extract_masked_texts,
};
use betterlite_guardrails::bedrock_message_selection::{
    content, prepare_for_role, reconcile, select,
};
use serde::Serialize;
use serde_json::to_value;
use wire::{Grounding, Input, Prepared, Selection, Step};

mod tests;
mod wire;
pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;
    fn run(input: &Self::Input) -> HelperResult {
        match &input.step {
            Step::Format(args) => format(args),
            Step::Grounding(args) => {
                let blocks = content::grounding(args.messages.as_deref(), args.contextual_fallback);
                encode(blocks.into_iter().map(Grounding::from).collect::<Vec<_>>())
            }
            Step::Selection(args) => encode(Selection::from(select(&args.texts, &args.context))),
            Step::Prepare(args) => encode(Prepared::from(prepare_for_role(
                args.messages.as_deref(),
                args.latest_only,
            ))),
            Step::Merge(args) => encode(reconcile::texts(
                &args.masked,
                &args.texts,
                args.scanned_slice,
                args.scanned_role_subset,
            )),
            Step::MergeMessages(args) => encode(reconcile::messages(
                args.original.as_deref(),
                &args.updated,
                args.indices.as_deref(),
            )),
            Step::Extract(args) => encode(extract_masked_texts(args.response.as_ref())),
            Step::MaskMessages(args) => {
                encode(apply_to_messages(&args.messages, &args.masked_texts))
            }
            Step::MaskResponse(args) => {
                let mut updated = args.response.clone();
                apply_to_response(&mut updated, &args.masked_texts);
                encode(updated)
            }
        }
    }
}

fn format(args: &wire::Format) -> HelperResult {
    let request = content::convert(
        args.source,
        args.messages.as_deref(),
        args.response.as_ref(),
        args.contextual_fallback,
    );
    encode(request)
}

fn encode<T>(value: T) -> HelperResult
where
    T: Serialize,
{
    let encoded = to_value(value);
    encoded.map_err(|error| {
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
