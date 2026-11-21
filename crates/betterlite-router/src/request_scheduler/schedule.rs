use super::error::SchedulerError;
use super::flow::FlowItem;
use super::service::RequestScheduler;
use crate::routing_timeouts::error::TimeoutError;
use crate::routing_timeouts::plan::TimeoutPlan;
use async_trait::async_trait;
use tokio::time::{Instant, sleep};
// Router.schedule_acompletion adds this header to a response the scheduler let through
pub const PRIORITIZATION_HEADER: &str = "x-litellm-request-prioritization-used";
// the error a scheduled call failed with, carrying the priority the scheduler stamped on it
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("the scheduled call answered with status {status}")]
pub struct ScheduledFailure {
    pub status: u16,
    pub code: Option<String>,
    pub priority: u8,
}
// B017's call behind a queued request: the scheduler decides when it starts and never wraps it again
#[async_trait]
pub trait ScheduledCall: Send + Sync {
    // Router._async_get_healthy_deployments().len() for this poll
    fn healthy_deployments(&self) -> usize;
    async fn call(&self) -> Result<(), ScheduledFailure>;
}
// What one scheduled request did: the queue let it run, and the response reports the prioritization header
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduleOutcome {
    pub request_id: String,
    pub priority: u8,
    pub polls: u32,
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
        let mut polls = 0_u32;
        loop {
            match plan.remaining(Instant::now().into_std()) {
                Ok(_remaining) => {}
                Err(TimeoutError::Cancelled) => {
                    return Err(self.cancelled(item).await);
                }
                Err(
                    TimeoutError::DeadlineExpired
                    | TimeoutError::AttemptExpired
                    | TimeoutError::NotFinite { .. }
                    | TimeoutError::Negative { .. }
                    | TimeoutError::OutOfRange { .. },
                ) => {
                    self.remove_request(&item.request_id, &item.model).await?;
                    return Err(SchedulerError::TimedOut {
                        model: item.model.clone(),
                        request_id: item.request_id.clone(),
                    });
                }
            }
            let allowed = self
                .poll(&item.request_id, &item.model, call.healthy_deployments())
                .await?;
            if allowed.value {
                break;
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
        // the queue entry leaves on every answer, the success path the pin leaks included
        let answered = call.call().await;
        self.remove_request(&item.request_id, &item.model).await?;
        match answered {
            Ok(()) => Ok(ScheduleOutcome {
                request_id: item.request_id.clone(),
                priority: item.priority,
                polls,
                prioritized: true,
            }),
            Err(mut failure) => {
                failure.priority = item.priority;
                Err(SchedulerError::Provider { failure })
            }
        }
    }
    // the timeout and the cancellation both take the waiting entry out before they answer
    async fn cancelled(&self, item: &FlowItem) -> SchedulerError {
        let removed = self.remove_request(&item.request_id, &item.model).await;
        match removed {
            Ok(_observed) => SchedulerError::Cancelled {
                request_id: item.request_id.clone(),
                model: item.model.clone(),
            },
            Err(error) => error,
        }
    }
}
