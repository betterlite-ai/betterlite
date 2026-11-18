use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::records::{
    KnownResponseEvent, ResponsesAPIStreamingResponse,
};
use betterlite_openai::api::responses_objects::ResponseApiUsage;

use crate::api::response_synthetic_events::billing::{
    BilledTerminal, UsageEstimate, billed_terminal_object, billed_terminal_response,
};
use crate::api::response_synthetic_events::estimate::{
    EstimateRequest, UsageTokenization, estimate_usage_safely,
};
use crate::api::response_synthetic_events::stamping::{
    ResponsesCostCalculator, stamp_object_usage_cost, stamp_usage_cost,
};

pub struct EstimatedUsage<'estimate, Tokenization> {
    pub tokenization: &'estimate Tokenization,
    pub model: &'estimate str,
    pub request_input: &'estimate Json,
    pub request: Json,
    pub generated: String,
}

impl<Tokenization> UsageEstimate for EstimatedUsage<'_, Tokenization>
where
    Tokenization: UsageTokenization,
{
    fn estimate(&self) -> Option<ResponseApiUsage> {
        estimate_usage_safely(
            self.tokenization,
            &EstimateRequest {
                model: self.model,
                request_input: self.request_input,
                responses_api_request: &self.request,
                generated_text: &self.generated,
            },
        )
    }
}

pub fn bill<Estimate, Calculator>(
    event: &mut ResponsesAPIStreamingResponse,
    estimate: Option<&Estimate>,
    calculator: Option<&Calculator>,
) where
    Estimate: UsageEstimate,
    Calculator: ResponsesCostCalculator,
{
    match event {
        ResponsesAPIStreamingResponse::Known(known) => match known.as_mut() {
            KnownResponseEvent::ResponseCompleted(record)
            | KnownResponseEvent::ResponseIncomplete(record) => {
                if let BilledTerminal::Estimated(copy) =
                    billed_terminal_response(&record.response, estimate)
                {
                    record.response = *copy;
                }
                stamp_usage_cost(&mut record.response, calculator);
            }
            KnownResponseEvent::ResponseFailed(record) => {
                stamp_usage_cost(&mut record.response, calculator);
            }
            KnownResponseEvent::ResponseCreated(_)
            | KnownResponseEvent::ResponseInProgress(_)
            | KnownResponseEvent::ResponsePartAdded(_)
            | KnownResponseEvent::ReasoningSummaryTextDelta(_)
            | KnownResponseEvent::ReasoningSummaryTextDone(_)
            | KnownResponseEvent::ReasoningSummaryPartDone(_)
            | KnownResponseEvent::OutputItemAdded(_)
            | KnownResponseEvent::OutputItemDone(_)
            | KnownResponseEvent::ContentPartAdded(_)
            | KnownResponseEvent::ContentPartDone(_)
            | KnownResponseEvent::OutputTextDelta(_)
            | KnownResponseEvent::OutputTextAnnotationAdded(_)
            | KnownResponseEvent::OutputTextDone(_)
            | KnownResponseEvent::RefusalDelta(_)
            | KnownResponseEvent::RefusalDone(_)
            | KnownResponseEvent::FunctionCallArgumentsDelta(_)
            | KnownResponseEvent::FunctionCallArgumentsDone(_)
            | KnownResponseEvent::FileSearchCallInProgress(_)
            | KnownResponseEvent::FileSearchCallSearching(_)
            | KnownResponseEvent::FileSearchCallCompleted(_)
            | KnownResponseEvent::WebSearchCallInProgress(_)
            | KnownResponseEvent::WebSearchCallSearching(_)
            | KnownResponseEvent::WebSearchCallCompleted(_)
            | KnownResponseEvent::McpListToolsInProgress(_)
            | KnownResponseEvent::McpListToolsCompleted(_)
            | KnownResponseEvent::McpListToolsFailed(_)
            | KnownResponseEvent::McpCallInProgress(_)
            | KnownResponseEvent::McpCallArgumentsDelta(_)
            | KnownResponseEvent::McpCallArgumentsDone(_)
            | KnownResponseEvent::McpCallCompleted(_)
            | KnownResponseEvent::McpCallFailed(_)
            | KnownResponseEvent::ImageGenerationPartialImage(_)
            | KnownResponseEvent::Error(_) => {}
        },
        ResponsesAPIStreamingResponse::Generic(record) => {
            let Some(response) = record.extra.get("response") else {
                return;
            };
            let Some(mut billed) = billed_terminal_object(response, estimate) else {
                return;
            };
            stamp_object_usage_cost(&mut billed, calculator);
            record
                .extra
                .insert("response".to_owned(), Json::Object(billed));
        }
    }
}
