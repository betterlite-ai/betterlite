use std::sync::{Arc, PoisonError};

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_models::decisions::Json;
use tokio::sync::{oneshot, watch};

use crate::concurrency_gauges::ConcurrencyGauges;
use crate::limiter_records::{ParallelSlotAcquisition, RateLimitDescriptor, RateLimitResponse};
use crate::pre_call_hook::common::{parallel_counter_keys, project_rate_limit};
use crate::pre_call_hook::{PreCallHookError, RateLimitPreCallHook, RequestCapacityGuard};
use crate::request_stash::RequestStashSlot;
use crate::token_estimation::RequestData;
use crate::windowed_admission::SlotIdGenerator as _;
use crate::windowed_admission::{
    AdmissionRequest, OK_CODE, OVER_LIMIT_CODE, WindowedAdmission, WindowedAdmissionError,
};

pub(super) async fn request_capacity(
    hook: &RateLimitPreCallHook,
    auth: &UserAPIKeyAuth,
    model: &str,
    request_data: Option<&RequestData>,
) -> Result<RequestCapacityGuard, PreCallHookError> {
    let data = request_data_with_model(request_data, model);
    let descriptors = hook
        .build_request_rate_limit_descriptors(auth, &data, None)
        .await?;
    let (guard, response) = start_admission(hook, &descriptors).await?;
    admit_outcome(hook, &descriptors, model, &response, guard)
}

async fn start_admission(
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
    let (status_tx, status_rx) = watch::channel(AdmissionStatus::Pending);
    spawn_admission(hook, admission_request, result_tx, status_tx);
    spawn_release(hook, finish_rx, status_rx, acquisition);
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
    status: watch::Sender<AdmissionStatus>,
) {
    let admission = Arc::clone(&hook.assembly.admission);
    hook.tasks.spawn(
        AdmissionTask {
            admission,
            request,
            result,
            status,
        }
        .run(),
    );
}

fn spawn_release(
    hook: &RateLimitPreCallHook,
    finish: oneshot::Receiver<()>,
    status: watch::Receiver<AdmissionStatus>,
    acquisition: ParallelSlotAcquisition,
) {
    let gauges = Arc::clone(&hook.assembly.gauges);
    hook.tasks.spawn(
        ReleaseTask {
            finish,
            status,
            gauges,
            acquisition,
        }
        .run(),
    );
}

fn admit_outcome(
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

fn request_data_with_model(request_data: Option<&RequestData>, model: &str) -> RequestData {
    let mut data = request_data.cloned().unwrap_or_default();
    data.insert("model".to_owned(), Json::Text(model.to_owned()));
    data
}

#[derive(Clone, Copy)]
enum AdmissionStatus {
    Pending,
    Finished { admitted: bool },
}

struct AdmissionTask {
    admission: Arc<WindowedAdmission>,
    request: AdmissionRequest,
    result: oneshot::Sender<Result<RateLimitResponse, WindowedAdmissionError>>,
    status: watch::Sender<AdmissionStatus>,
}

impl AdmissionTask {
    async fn run(self) {
        let outcome = self.admission.should_rate_limit(&self.request).await;
        let admitted = outcome
            .as_ref()
            .is_ok_and(|response| response.overall_code == OK_CODE);
        if self
            .status
            .send(AdmissionStatus::Finished { admitted })
            .is_err()
        {
            tracing::debug!("the request capacity release task ended before the admission");
        }
        if self.result.send(outcome).is_err() {
            tracing::debug!("the request capacity caller stopped before the admission ended");
        }
    }
}

struct ReleaseTask {
    finish: oneshot::Receiver<()>,
    status: watch::Receiver<AdmissionStatus>,
    gauges: Arc<ConcurrencyGauges>,
    acquisition: ParallelSlotAcquisition,
}

impl ReleaseTask {
    async fn run(mut self) {
        if self.finish.await.is_err() {
            tracing::debug!("the request capacity guard ended without its finish signal");
        }
        if admitted(&mut self.status).await {
            release_capacity_slot(&self.gauges, &self.acquisition).await;
        }
    }
}

async fn admitted(status: &mut watch::Receiver<AdmissionStatus>) -> bool {
    match status
        .wait_for(|value| !matches!(value, AdmissionStatus::Pending))
        .await
    {
        Ok(value) => matches!(*value, AdmissionStatus::Finished { admitted: true }),
        Err(_) => false,
    }
}

async fn release_capacity_slot(gauges: &ConcurrencyGauges, acquisition: &ParallelSlotAcquisition) {
    let slot = RequestStashSlot::default();
    let stash = slot.get_or_create();
    {
        let mut state = stash.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.parallel_slot = Some(acquisition.clone());
    }
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
