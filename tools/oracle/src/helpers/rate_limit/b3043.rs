mod fakes;

use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError};
use std::time::{Duration, UNIX_EPOCH};

use betterlite_models::decisions::Json;
use betterlite_rate_limit::concurrency_gauges::ConcurrencyGauges;
use betterlite_rate_limit::fail_closed::FailClosedPolicy;
use betterlite_rate_limit::limiter_records::{
    ParallelSlotAcquisition, RateLimitDescriptor, RateLimitResponse,
};
use betterlite_rate_limit::model_descriptors::ModelDescriptorBuilder;
use betterlite_rate_limit::project_admission::{
    EstimationServices, ProjectAdmission, ProjectAdmissionError, ProjectIoRequest, TpmReservation,
};
use betterlite_rate_limit::provider_attribution::RateLimitErrorAttribution;
use betterlite_rate_limit::rate_limit_window_store::WindowStore;
use betterlite_rate_limit::request_stash::RequestStashSlot;
use betterlite_rate_limit::settlement_and_rollback::AtomicSettlement;
use betterlite_rate_limit::token_adjustments::store::TokenAdjustments;
use betterlite_rate_limit::token_estimation::RequestData;
use betterlite_rate_limit::token_estimation::output_cap::CapEndpoint;
use betterlite_rate_limit::token_reservations::TokenReservations;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;
use tokio::sync::Mutex;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

use crate::b3042::CounterValueInput;
use fakes::{Clock, NoGroups, NoProviders, Recorder, Uncounted};

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    descriptors: Vec<RateLimitDescriptor>,
    data: Json,
    requested_model: Option<String>,
    call_type: Option<String>,
    tpm_reservation_scopes: Vec<(String, String)>,
    tpm_reservation_amount: i64,
    window_size: i64,
    now: u64,
    cache: BTreeMap<String, CounterValueInput>,
    parallel_slot: Option<ParallelSlotAcquisition>,
    rate_limit_response: Option<RateLimitResponse>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let recorder = Arc::new(Recorder::new(&input.cache));
        let slot = RequestStashSlot::default();
        let stash = slot.get_or_create();
        let mut state = stash.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.parallel_slot.clone_from(&input.parallel_slot);
        state
            .rate_limit_response
            .clone_from(&input.rate_limit_response);
        drop(state);
        // LiteLLM returns at once for a body that is not a dict, and betterlite's typed body can't be one
        let (raised, data) = match &input.data {
            Json::Object(fields) => {
                let mut body = fields.clone();
                let result = admit(input, &recorder, &slot, &mut body)?;
                (raised_json(result)?, Json::Object(body))
            }
            other @ (Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::Text(_)
            | Json::List(_)) => (Value::Null, other.clone()),
        };
        Ok(json!({
            "raised": raised,
            "data": data,
            "stash": stash_json(&slot),
            "effects": recorder.effects_json(),
        }))
    }
}

struct Stores {
    adjustments: TokenAdjustments,
    gauges: ConcurrencyGauges,
    descriptors: ModelDescriptorBuilder,
    clock: Arc<Clock>,
}

impl Stores {
    fn build(recorder: &Arc<Recorder>, input: &Input) -> Result<Self, Box<HelperError>> {
        let now = UNIX_EPOCH
            .checked_add(Duration::from_secs(input.now))
            .ok_or("the case time is too large")
            .or_value_error()?;
        let clock = Arc::new(Clock(now));
        let gauges = ConcurrencyGauges {
            cache: Arc::<Recorder>::clone(recorder),
            clock: Arc::<Clock>::clone(&clock),
            acquire_script: None,
            release_script: None,
            count_script: None,
            release_batch: None,
            fail_closed: FailClosedPolicy { enabled: false },
            check_and_increment_lock: Mutex::new(()),
        };
        let windows = WindowStore {
            cache: Arc::<Recorder>::clone(recorder),
            rate_limiter_script: None,
            counter_read_script: None,
            request_batch: None,
            check_and_increment_lock: Mutex::new(()),
        };
        Ok(Self {
            adjustments: TokenAdjustments {
                windows,
                usage: Arc::<Recorder>::clone(recorder),
                post_call_batch: None,
                token_increment_script: None,
                guarded_token_script: None,
            },
            gauges,
            descriptors: ModelDescriptorBuilder {
                window_size: input.window_size,
                resolver: Box::new(NoGroups),
            },
            clock,
        })
    }

    fn admission(&self, window_size: i64) -> ProjectAdmission<'_> {
        ProjectAdmission {
            reservations: TokenReservations {
                settlement: AtomicSettlement {
                    windows: &self.adjustments.windows,
                    clock: self.clock.as_ref(),
                    script: None,
                    refund_script: None,
                    fail_closed: FailClosedPolicy { enabled: false },
                    default_window_size: window_size,
                },
                adjustments: &self.adjustments,
                descriptors: &self.descriptors,
                attribution: RateLimitErrorAttribution {
                    requested_model: None,
                    native: &NoProviders,
                    router: None,
                },
            },
            gauges: &self.gauges,
            estimation: EstimationServices {
                translation: &Uncounted,
                responses: &Uncounted,
                counter: &Uncounted,
            },
        }
    }
}

fn admit(
    input: &Input,
    recorder: &Arc<Recorder>,
    slot: &RequestStashSlot,
    body: &mut RequestData,
) -> Result<Result<(), ProjectAdmissionError>, Box<HelperError>> {
    let stores = Stores::build(recorder, input)?;
    let request = ProjectIoRequest {
        descriptors: &input.descriptors,
        requested_model: input.requested_model.as_deref(),
        call_type: input.call_type.as_deref(),
        endpoint: CapEndpoint::Generic,
        tpm_reservation: TpmReservation {
            scopes: &input.tpm_reservation_scopes,
            amount: input.tpm_reservation_amount,
        },
    };
    let runtime = Builder::new_current_thread().build()?;
    Ok(runtime.block_on(
        stores
            .admission(input.window_size)
            .reserve_project_io_tokens(&request, body, slot),
    ))
}

fn raised_json(result: Result<(), ProjectAdmissionError>) -> HelperResult {
    let rejection = match result {
        Ok(()) => return Ok(Value::Null),
        Err(ProjectAdmissionError::RateLimited(rejection)) => rejection,
        Err(
            other @ (ProjectAdmissionError::Reservation(_) | ProjectAdmissionError::Projection(_)),
        ) => {
            return Err(other).or_value_error();
        }
    };
    let mut headers: Option<BTreeMap<&str, &str>> = None;
    for (name, value) in rejection.headers.iter().flatten() {
        let text = value.to_str().or_value_error()?;
        headers.get_or_insert_default().insert(name.as_str(), text);
    }
    Ok(json!({
        "status": 429_u16,
        "headers": headers,
        "rate_limit_type": rejection.rate_limit_type,
        "category": rejection.category,
    }))
}

fn stash_json(slot: &RequestStashSlot) -> Value {
    let Some(stash) = slot.get() else {
        return Value::Null;
    };
    let state = stash.state.lock().unwrap_or_else(PoisonError::into_inner);
    let stored = state.rate_limit_response.as_ref().map(|response| {
        json!({
            "overall_code": &response.overall_code,
            "statuses": &response.statuses,
            "reservation_windows": response.reservation_windows.iter().flatten().collect::<Vec<_>>(),
        })
    });
    let value = json!({
        "owner_litellm_call_id": &state.owner_litellm_call_id,
        "reservation_released": state.reservation_released,
        "parallel_slot_released": state.parallel_slot.is_none(),
        "itpm_reserved_tokens": state.itpm_reserved_tokens,
        "itpm_reserved_scopes": &state.itpm_reserved_scopes,
        "itpm_reserved_windows": &state.itpm_reserved_windows,
        "otpm_reserved_tokens": state.otpm_reserved_tokens,
        "otpm_reserved_scopes": &state.otpm_reserved_scopes,
        "otpm_reserved_windows": &state.otpm_reserved_windows,
        "rate_limit_response": stored,
    });
    drop(state);
    value
}
