use super::error::SchedulerError;
use super::flow::FlowItem;
use super::queue;
use crate::routing_state_store::RoutingStateStore;
use crate::routing_state_store::error::Observed;
use crate::routing_value_store::{RoutingValueStore, SchedulerTuple};
use betterlite_config::sections::request_scheduler::Settings;
use std::sync::{Arc, Mutex};
use std::time::Duration;
// Router.schedule_acompletion's `str(uuid.uuid4())`, injected so a test or a probe decides the id
pub type RequestIdSource = Arc<dyn Fn() -> String + Send + Sync>;
// a shared queue changes under a compare-and-set; another worker may win a few times before this one does
pub(super) const MAX_CAS_ATTEMPTS: u32 = 8;
// Router.scheduler over B360's store: this worker's queue, and the shared queue when Redis is configured
pub struct RequestScheduler {
    pub(super) local: RoutingValueStore,
    pub(super) shared: Option<RoutingStateStore>,
    pub(super) settings: Settings,
    pub(super) local_lock: Mutex<()>,
    request_ids: RequestIdSource,
}
impl RequestScheduler {
    #[must_use]
    pub fn new(
        local: RoutingValueStore,
        shared: Option<RoutingStateStore>,
        settings: Settings,
        request_ids: RequestIdSource,
    ) -> Self {
        Self {
            local,
            shared,
            settings,
            local_lock: Mutex::new(()),
            request_ids,
        }
    }
    #[must_use]
    pub fn polling_interval(&self) -> Duration {
        if let Ok(interval) = Duration::try_from_secs_f64(self.settings.polling_interval) {
            return interval;
        }
        Duration::ZERO
    }
    #[must_use]
    pub fn priority(&self, requested: Option<u8>) -> Option<u8> {
        if let Some(priority) = requested {
            return Some(priority);
        }
        self.settings.default_priority
    }
    pub fn next_request_id(&self) -> Result<String, SchedulerError> {
        let generated = (self.request_ids)();
        if !queue::valid_request_id(&generated) {
            return Err(SchedulerError::InvalidRequestId { id: generated });
        }
        Ok(generated)
    }
    // Scheduler.add_request over the bound this task adds: a full queue refuses the request instead of growing
    pub async fn add_request(&self, item: &FlowItem) -> Result<Observed<()>, SchedulerError> {
        if !queue::valid_request_id(&item.request_id) {
            return Err(SchedulerError::InvalidRequestId {
                id: item.request_id.clone(),
            });
        }
        let model = item.model.as_str();
        let changed = self
            .change(model, |members| {
                if members.len() >= queue::MAX_PENDING_REQUESTS {
                    return Err(SchedulerError::QueueFull {
                        model: model.to_owned(),
                        limit: queue::MAX_PENDING_REQUESTS,
                    });
                }
                let entry = SchedulerTuple {
                    priority: item.priority,
                    request_id: item.request_id.clone(),
                };
                let mut next = members;
                next.extend(queue::encoded(&[entry]));
                Ok(next)
            })
            .await?;
        Ok(Observed {
            value: (),
            warning: changed.warning,
        })
    }
    pub async fn remove_request(&self, item: &FlowItem) -> Result<Observed<()>, SchedulerError> {
        let changed = self
            .change(&item.model, |members| {
                Ok(queue::without(&members, &item.request_id))
            })
            .await?;
        Ok(Observed {
            value: (),
            warning: changed.warning,
        })
    }
    // Scheduler.get_queue: the model's queue in the order a heap would serve it
    pub async fn queue(
        &self,
        model: &str,
    ) -> Result<Observed<Vec<SchedulerTuple>>, SchedulerError> {
        let members = self.queue_members(model).await?;
        Ok(Observed {
            value: queue::scheduled(&members.value),
            warning: members.warning,
        })
    }
    // Scheduler.peek compares the head without taking it
    pub async fn peek(
        &self,
        request_id: &str,
        model: &str,
    ) -> Result<Observed<bool>, SchedulerError> {
        let queued = self.queue_members(model).await?;
        match queue::head(&queued.value) {
            Some(head) => Ok(Observed {
                value: head.request_id == request_id,
                warning: queued.warning,
            }),
            None => Err(SchedulerError::EmptyQueue {
                model: model.to_owned(),
            }),
        }
    }
    // Scheduler.poll: a healthy deployment serves any request, and without one only the queue's head runs
    pub async fn poll(
        &self,
        request_id: &str,
        model: &str,
        healthy_deployments: usize,
    ) -> Result<Observed<bool>, SchedulerError> {
        let queued = self.queue_members(model).await?;
        if queued.value.is_empty() {
            return Err(SchedulerError::EmptyQueue {
                model: model.to_owned(),
            });
        }
        if healthy_deployments > 0 {
            return Ok(Observed {
                value: true,
                warning: queued.warning,
            });
        }
        let head = queue::head(&queued.value).ok_or_else(|| SchedulerError::EmptyQueue {
            model: model.to_owned(),
        })?;
        if head.request_id != request_id {
            return Ok(Observed {
                value: false,
                warning: queued.warning,
            });
        }
        let removed = self
            .change(model, |members| Ok(queue::without(&members, request_id)))
            .await?;
        Ok(Observed {
            value: true,
            warning: removed.warning.or(queued.warning),
        })
    }
}
