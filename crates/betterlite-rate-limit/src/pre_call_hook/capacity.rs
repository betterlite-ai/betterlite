use std::sync::{Arc, PoisonError};

use betterlite_models::decisions::Json;
use tokio::sync::oneshot;

use crate::concurrency_gauges::ConcurrencyGauges;
use crate::limiter_records::{ParallelSlotAcquisition, RateLimitDescriptor, RateLimitResponse};
use crate::pre_call_hook::common::{parallel_counter_keys, project_rate_limit};
use crate::pre_call_hook::{PreCallHookError, RateLimitPreCallHook, RequestCapacityGuard};
use crate::request_stash::RequestStashSlot;
use crate::token_estimation::RequestData;
use crate::windowed_admission::{
    AdmissionRequest, OK_CODE, OVER_LIMIT_CODE, WindowedAdmission, WindowedAdmissionError,
};

pub(super) async fn start_admission(
    hook: &RateLimitPreCallHook,
    descriptors: &[RateLimitDescriptor],
) -> Result<(RequestCapacityGuard, RateLimitResponse), PreCallHookError> {
    let tasks = start_tasks(hook, descriptors);
    let response = match tasks.result.await {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => return Err(PreCallHookError::Admission(error)),
        Err(_) => return Err(PreCallHookError::AdmissionTaskLost),
    };
    Ok((tasks.guard, response))
}

// the guard and the admission result, with both background tasks already tracking their work
struct CapacityTasks {
    guard: RequestCapacityGuard,
    result: oneshot::Receiver<Result<RateLimitResponse, WindowedAdmissionError>>,
}

fn start_tasks(hook: &RateLimitPreCallHook, descriptors: &[RateLimitDescriptor]) -> CapacityTasks {
    let counter_keys = parallel_counter_keys(descriptors);
    let slot_id = hook.assembly.admission.slot_ids.next_slot_id();
    let acquisition = ParallelSlotAcquisition {
        slot_id: slot_id.clone(),
        counter_keys,
    };
    let admission_request = AdmissionRequest {
        descriptors: descriptors.to_vec(),
        parent_otel_span: None,
        read_only: false,
        skip_tpm_check: true,
        parallel_slot_id: Some(slot_id),
    };
    let (result_tx, result_rx) = oneshot::channel();
    let (finish_tx, finish_rx) = oneshot::channel();
    let (admitted_tx, admitted_rx) = oneshot::channel();
    spawn_admission(hook, admission_request, result_tx, admitted_tx);
    spawn_release(hook, finish_rx, admitted_rx, acquisition);
    CapacityTasks {
        guard: RequestCapacityGuard {
            finish: Some(finish_tx),
        },
        result: result_rx,
    }
}

fn spawn_admission(
    hook: &RateLimitPreCallHook,
    request: AdmissionRequest,
    result: oneshot::Sender<Result<RateLimitResponse, WindowedAdmissionError>>,
    admitted: oneshot::Sender<bool>,
) {
    let admission = Arc::clone(&hook.assembly.admission);
    hook.tasks.spawn(
        AdmissionTask {
            admission,
            request,
            result,
            admitted,
        }
        .run(),
    );
}

fn spawn_release(
    hook: &RateLimitPreCallHook,
    finish: oneshot::Receiver<()>,
    admitted: oneshot::Receiver<bool>,
    acquisition: ParallelSlotAcquisition,
) {
    let gauges = Arc::clone(&hook.assembly.gauges);
    hook.tasks.spawn(
        ReleaseTask {
            finish,
            admitted,
            gauges,
            acquisition,
        }
        .run(),
    );
}

pub(super) fn admit_outcome(
    hook: &RateLimitPreCallHook,
    descriptors: &[RateLimitDescriptor],
    model: &str,
    response: &RateLimitResponse,
    guard: RequestCapacityGuard,
) -> Result<RequestCapacityGuard, PreCallHookError> {
    if response.overall_code != OVER_LIMIT_CODE {
        return Ok(guard);
    }
    let Some(error) = project_rate_limit(hook, response, descriptors, Some(model))? else {
        return Ok(guard);
    };
    Err(PreCallHookError::RateLimited(Box::new(error)))
}

pub(super) fn request_data_with_model(
    request_data: Option<&RequestData>,
    model: &str,
) -> RequestData {
    let mut data = request_data.cloned().unwrap_or_default();
    data.insert("model".to_owned(), Json::Text(model.to_owned()));
    data
}

struct AdmissionTask {
    admission: Arc<WindowedAdmission>,
    request: AdmissionRequest,
    result: oneshot::Sender<Result<RateLimitResponse, WindowedAdmissionError>>,
    admitted: oneshot::Sender<bool>,
}

impl AdmissionTask {
    async fn run(self) {
        let outcome = self.admission.should_rate_limit(&self.request).await;
        let admitted = outcome
            .as_ref()
            .is_ok_and(|response| response.overall_code == OK_CODE);
        if self.admitted.send(admitted).is_err() {
            tracing::debug!("the request capacity release task ended before the admission");
        }
        if self.result.send(outcome).is_err() {
            tracing::debug!("the request capacity caller stopped before the admission ended");
        }
    }
}

struct ReleaseTask {
    finish: oneshot::Receiver<()>,
    admitted: oneshot::Receiver<bool>,
    gauges: Arc<ConcurrencyGauges>,
    acquisition: ParallelSlotAcquisition,
}

impl ReleaseTask {
    async fn run(self) {
        // the guard sends this signal as it drops; one that ended without sending still holds its slot
        if self.finish.await.is_err() {
            tracing::debug!("the request capacity guard ended without its finish signal");
            return;
        }
        // a lost verdict means the admission ended without one, so the slot is not released
        if self.admitted.await.unwrap_or(false) {
            release_capacity_slot(&self.gauges, &self.acquisition).await;
        }
    }
}

async fn release_capacity_slot(gauges: &ConcurrencyGauges, acquisition: &ParallelSlotAcquisition) {
    let slot = RequestStashSlot::default();
    let stash = slot.get_or_create();
    stash
        .state
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .parallel_slot = Some(acquisition.clone());
    gauges
        .release_stashed_parallel_slot(Some(&stash), false)
        .await;
}

impl Drop for RequestCapacityGuard {
    fn drop(&mut self) {
        let Some(finish) = self.finish.take() else {
            return;
        };
        if finish.send(()).is_err() {
            tracing::debug!("the request capacity release task ended before its guard");
        }
    }
}
