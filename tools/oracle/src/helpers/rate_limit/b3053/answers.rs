use std::collections::BTreeMap;
use std::mem::take;
use std::sync::{Arc, PoisonError};

use betterlite_logging::fields::payload::records::StandardLoggingMetadata;
use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_rate_limit::concurrency_gauges::ConcurrencyGauges;
use betterlite_rate_limit::dynamic_callbacks::{
    LogSuccessEvent, PriorityMetadata, log_success_event, post_call_success_hook,
};
use betterlite_rate_limit::dynamic_priority::settings::{PriorityPolicy, PrioritySettings};
use betterlite_rate_limit::fail_closed::FailClosedPolicy;
use betterlite_rate_limit::limiter_records::RateLimitResponse;
use betterlite_rate_limit::post_call_settlement::settlement::PostCallSettlement;
use betterlite_rate_limit::post_call_settlement::success_hook::SuccessHook;
use betterlite_rate_limit::rate_limit_window_store::WindowStore;
use betterlite_rate_limit::reconciliation_operations::success_event::SuccessResponse;
use betterlite_rate_limit::request_stash::RequestStashSlot;
use betterlite_rate_limit::token_adjustments::store::{TokenAdjustments, UsageIncrements};
use betterlite_rate_limit::usage_resolution::ResponseObject;
use betterlite_redis::cache_primitives::pipeline_increment::IncrementOperation;
use serde_json::{Value, from_value, json};
use tokio::runtime::Builder;
use tokio::sync::Mutex as AsyncMutex;
use tracing::subscriber::set_default;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::registry;

use super::fakes::{
    EpochClock, PriorityLogs, RecordedPipelines, SilentCounters, SilentGauges, SilentStore,
};
use super::{LogSuccessCase, PostCallCase};

struct Fixture {
    gauges: ConcurrencyGauges,
    adjustments: TokenAdjustments,
    store: SilentStore,
    usage: Arc<RecordedPipelines>,
}

fn dependencies() -> Fixture {
    let usage = Arc::new(RecordedPipelines::default());
    let increments: Arc<dyn UsageIncrements> = Arc::<RecordedPipelines>::clone(&usage);
    Fixture {
        gauges: ConcurrencyGauges {
            cache: Arc::new(SilentGauges),
            clock: Arc::new(EpochClock),
            acquire_script: None,
            release_script: None,
            count_script: None,
            release_batch: None,
            fail_closed: FailClosedPolicy { enabled: false },
            check_and_increment_lock: AsyncMutex::new(()),
        },
        adjustments: TokenAdjustments {
            windows: WindowStore {
                cache: Arc::new(SilentCounters),
                rate_limiter_script: None,
                counter_read_script: None,
                request_batch: None,
                check_and_increment_lock: AsyncMutex::new(()),
            },
            usage: increments,
            post_call_batch: None,
            token_increment_script: None,
            guarded_token_script: None,
        },
        store: SilentStore,
        usage,
    }
}

impl Fixture {
    fn settlement(&self) -> PostCallSettlement<'_> {
        PostCallSettlement {
            gauges: &self.gauges,
            adjustments: &self.adjustments,
            batch_store: &self.store,
        }
    }
}

pub(crate) fn post_call_answer(case: &PostCallCase) -> HelperResult {
    let runtime = Builder::new_current_thread().build()?;
    let headers = runtime.block_on(post_call_run(case));
    Ok(json!({"headers": headers}))
}

async fn post_call_run(case: &PostCallCase) -> Option<AdditionalHeaders> {
    if case.probe_raises {
        return None;
    }
    let fixture = dependencies();
    let slot = install_stash(case);
    let hook = SuccessHook {
        stash: &slot,
        call_id: None,
        response: None,
    };
    let priority = PriorityMetadata {
        team_metadata: case.team_metadata.as_ref(),
        key_metadata: &case.key_metadata,
    };
    let mut headers = case.hidden_params.then(|| headers_map(&case.headers));
    post_call_success_hook(&fixture.settlement(), &hook, &priority, headers.as_mut()).await;
    headers
}

fn install_stash(case: &PostCallCase) -> RequestStashSlot {
    let slot = RequestStashSlot::default();
    let stashed = slot.get_or_create();
    let mut state = stashed.state.lock().unwrap_or_else(PoisonError::into_inner);
    state.rate_limit_response = Some(RateLimitResponse {
        overall_code: "OK".to_owned(),
        statuses: case.statuses.clone(),
        reservation_windows: None,
    });
    drop(state);
    slot
}

fn headers_map(entries: &BTreeMap<String, Json>) -> AdditionalHeaders {
    entries
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

pub(crate) fn log_success_answer(case: &LogSuccessCase) -> HelperResult {
    let logs = PriorityLogs::default();
    let runtime = Builder::new_current_thread().build()?;
    let guard = set_default(registry().with(logs.clone()));
    let pipelines = runtime.block_on(log_success_run(case))?;
    drop(guard);
    let priority_logged = logs
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .last()
        .cloned();
    Ok(json!({
        "pipelines": pipelines_json(&pipelines),
        "priority_logged": priority_logged,
    }))
}

async fn log_success_run(
    case: &LogSuccessCase,
) -> Result<Vec<Vec<IncrementOperation>>, Box<HelperError>> {
    let fixture = dependencies();
    let wire = json!({"user_api_key_auth_metadata": &case.auth_metadata});
    let metadata: StandardLoggingMetadata = from_value(wire).or_value_error()?;
    let response = response_object(case);
    let event = LogSuccessEvent {
        metadata: &metadata,
        model_group: case.model_group.as_deref(),
        response: SuccessResponse::Modelled(&response),
        rate_limit_type: case.rate_limit_type,
        window_size: case.window_size,
    };
    log_success_event(&fixture.adjustments, &policy(case), &event).await;
    let mut lines = fixture
        .usage
        .lines
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    Ok(take(&mut *lines))
}

fn response_object(case: &LogSuccessCase) -> ResponseObject {
    case.usage
        .as_ref()
        .map_or(ResponseObject::WithUsage(None), |usage| {
            ResponseObject::Usage(usage.clone())
        })
}

fn policy(case: &LogSuccessCase) -> PriorityPolicy {
    PriorityPolicy {
        settings: PrioritySettings::default(),
        reservation: case.priority_reservation.clone(),
    }
}

fn pipelines_json(pipelines: &[Vec<IncrementOperation>]) -> Vec<Value> {
    pipelines
        .iter()
        .map(|operations| Value::Array(operations.iter().map(operation_json).collect()))
        .collect()
}

fn operation_json(operation: &IncrementOperation) -> Value {
    json!({
        "key": &operation.key,
        "increment_value": operation.increment_value,
        "ttl": operation.ttl,
    })
}
