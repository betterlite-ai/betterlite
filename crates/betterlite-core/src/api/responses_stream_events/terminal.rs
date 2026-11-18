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
    pub request: EstimateRequest<'estimate>,
}

impl<Tokenization> UsageEstimate for EstimatedUsage<'_, Tokenization>
where
    Tokenization: UsageTokenization,
{
    fn estimate(&self) -> Option<ResponseApiUsage> {
        estimate_usage_safely(self.tokenization, &self.request)
    }
}

// completed and incomplete bill the response, failed only logs it
#[derive(Clone, Copy)]
pub enum Terminal {
    Billed,
    Failed,
}

impl Terminal {
    #[must_use]
    pub fn of(event: &ResponsesAPIStreamingResponse) -> Option<Self> {
        match event {
            ResponsesAPIStreamingResponse::Known(known) => {
                if let KnownResponseEvent::ResponseFailed(_) = known.as_ref() {
                    return Some(Self::Failed);
                }
                if let KnownResponseEvent::ResponseCompleted(_)
                | KnownResponseEvent::ResponseIncomplete(_) = known.as_ref()
                {
                    return Some(Self::Billed);
                }
                None
            }
            ResponsesAPIStreamingResponse::Generic(record) => match record.r#type.as_str() {
                "response.failed" => Some(Self::Failed),
                "response.completed" | "response.incomplete" => Some(Self::Billed),
                _ => None,
            },
        }
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
        ResponsesAPIStreamingResponse::Known(known) => {
            if let KnownResponseEvent::ResponseCompleted(record)
            | KnownResponseEvent::ResponseIncomplete(record) = known.as_mut()
            {
                if let BilledTerminal::Estimated(copy) =
                    billed_terminal_response(&record.response, estimate)
                {
                    record.response = *copy;
                }
                stamp_usage_cost(&mut record.response, calculator);
            }
            if let KnownResponseEvent::ResponseFailed(record) = known.as_mut() {
                stamp_usage_cost(&mut record.response, calculator);
            }
        }
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
