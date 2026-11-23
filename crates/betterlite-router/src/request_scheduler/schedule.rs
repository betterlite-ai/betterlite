use super::error::SchedulerError;
use super::flow::FlowItem;
use crate::router_core::headers;
use async_trait::async_trait;
use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;
// Router.schedule_acompletion adds this header to a response the scheduler let through
pub const PRIORITIZATION_HEADER: &str = "x-litellm-request-prioritization-used";
// Router.schedule_acompletion's `_hidden_params["additional_headers"]` update, and whether the response kept it
#[must_use]
pub fn stamp_prioritization(response: headers::Response) -> (headers::Response, bool) {
    let Some(mut prepared) = headers::prepare_response(response) else {
        return (headers::Response::Absent, false);
    };
    headers::ensure_additional_headers(&mut prepared)
        .insert(PRIORITIZATION_HEADER.to_owned(), Json::Bool(true));
    let prioritized = headers::response_has_hidden_params(&prepared);
    (headers::write_hidden_params(prepared), prioritized)
}
// B017's call behind a queued request: the scheduler decides when it starts and never wraps it again
#[async_trait]
pub trait ScheduledCall: Send + Sync {
    // Router._async_get_healthy_deployments().len() for this poll
    fn healthy_deployments(&self) -> usize;
    async fn call(&self) -> Result<headers::Response, Box<ErrorClass>>;
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduleOutcome {
    pub request_id: String,
    pub priority: u8,
    pub polls: u32,
    pub response: headers::Response,
    pub prioritized: bool,
}
pub(super) fn finish(
    item: &FlowItem,
    polls: u32,
    answered: Result<headers::Response, Box<ErrorClass>>,
) -> Result<ScheduleOutcome, SchedulerError> {
    match answered {
        Ok(response) => {
            let (stamped, prioritized) = stamp_prioritization(response);
            Ok(ScheduleOutcome {
                request_id: item.request_id.clone(),
                priority: item.priority,
                polls,
                response: stamped,
                prioritized,
            })
        }
        Err(error) => Err(SchedulerError::Provider {
            error,
            priority: item.priority,
        }),
    }
}
