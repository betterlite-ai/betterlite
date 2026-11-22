use super::error::SchedulerError;
use super::flow::FlowItem;
use super::service::RequestScheduler;
use crate::router_core::headers;
use crate::routing_timeouts::error::TimeoutError;
use crate::routing_timeouts::plan::TimeoutPlan;
use async_trait::async_trait;
use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;
use tokio::time::{Instant, sleep};
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
impl RequestScheduler {
    // Router.schedule_acompletion, with the routing deadline and the caller's cancellation instead of Router.timeout
    pub async fn schedule<C>(
        &self,
        item: &FlowItem,
        call: &C,
        plan: &TimeoutPlan,
    ) -> Result<ScheduleOutcome, SchedulerError>
    where
        C: ScheduledCall,
    {
        self.add_request(item).await?;
        // the poll loop nests more generator states than rustc lays out by default, so its future lives on the heap
        let polls = Box::pin(self.wait_for_turn(item, call, plan)).await?;
        let answered = call.call().await;
        if let Err(error) = self.remove_request(item).await {
            tracing::warn!(%error, "the completed request left a scheduler queue entry behind");
        }
        finish(item, polls, answered)
    }
    async fn wait_for_turn<C>(
        &self,
        item: &FlowItem,
        call: &C,
        plan: &TimeoutPlan,
    ) -> Result<u32, SchedulerError>
    where
        C: ScheduledCall,
    {
        let mut polls = 0_u32;
        loop {
            match plan.remaining(Instant::now().into_std()) {
                Ok(_remaining) => {}
                Err(TimeoutError::Cancelled) => return Err(self.cancelled(item).await),
                Err(_expired) => {
                    if let Err(error) = self.remove_request(item).await {
                        tracing::warn!(%error, "the expired request left a scheduler queue entry behind");
                    }
                    return Err(SchedulerError::TimedOut {
                        model: item.model.clone(),
                        request_id: item.request_id.clone(),
                    });
                }
            }
            let allowed =
                Box::pin(self.poll(&item.request_id, &item.model, call.healthy_deployments()))
                    .await?;
            if allowed.value {
                return Ok(polls);
            }
            polls = polls.saturating_add(1);
            let slept = plan
                .cancellation
                .run_until_cancelled(sleep(self.polling_interval()))
                .await;
            if slept.is_none() {
                return Err(self.cancelled(item).await);
            }
        }
    }
    async fn cancelled(&self, item: &FlowItem) -> SchedulerError {
        let removed = self.remove_request(item).await;
        match removed {
            Ok(_observed) => SchedulerError::Cancelled {
                request_id: item.request_id.clone(),
                model: item.model.clone(),
            },
            Err(error) => error,
        }
    }
}
fn finish(
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
