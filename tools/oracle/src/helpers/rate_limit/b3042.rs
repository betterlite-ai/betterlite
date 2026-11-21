mod answers;
mod fakes;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_rate_limit::fail_closed::FailClosedPolicy;
use betterlite_rate_limit::limiter_records::{
    CacheCounterValue, RateLimitDescriptor, RateLimitResponse, ReservationWindowIdentity,
};
use betterlite_rate_limit::model_descriptors::ModelDescriptorBuilder;
use betterlite_rate_limit::provider_attribution::RateLimitErrorAttribution;
use betterlite_rate_limit::rate_limit_window_store::WindowStore;
use betterlite_rate_limit::settlement_and_rollback::AtomicSettlement;
use betterlite_rate_limit::token_adjustments::store::TokenAdjustments;
use betterlite_rate_limit::token_reservations::{TokenReservationError, TokenReservations};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};
use tokio::sync::Mutex;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

use answers::{effects_json, raised_json, response_json};
use fakes::{Effects, FixedClock, GroupMap, RecordingCache, RecordingUsage, UnresolvedProviders};

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum CounterValueInput {
    Int(i64),
    Float(f64),
    Text(String),
}

impl CounterValueInput {
    fn counter_value(&self) -> CacheCounterValue {
        match self {
            Self::Int(number) => CacheCounterValue::Int(*number),
            Self::Float(number) => CacheCounterValue::Float(*number),
            Self::Text(text) => CacheCounterValue::Text(text.clone()),
        }
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct ReserveTpmInput {
    descriptors: Vec<RateLimitDescriptor>,
    estimated_tokens: i64,
    window_size: i64,
    now: u64,
    #[serde(default)]
    groups: BTreeMap<String, String>,
    #[serde(default)]
    cache: BTreeMap<String, CounterValueInput>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RefundReservedInput {
    scopes: Vec<(String, String)>,
    amount: i64,
    #[serde(default)]
    reservation_windows: Vec<ReservationWindowIdentity>,
    window_size: i64,
    now: u64,
    #[serde(default)]
    groups: BTreeMap<String, String>,
    #[serde(default)]
    cache: BTreeMap<String, CounterValueInput>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct IoReserveInput {
    descriptors: Vec<RateLimitDescriptor>,
    estimated_input_tokens: i64,
    estimated_output_tokens: i64,
    window_size: i64,
    now: u64,
    #[serde(default)]
    groups: BTreeMap<String, String>,
    #[serde(default)]
    cache: BTreeMap<String, CounterValueInput>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct FrameInput {
    #[serde(default)]
    auth: Option<BTreeMap<String, JsonData>>,
    #[serde(default)]
    requested_model: Option<String>,
    estimated_input_tokens: i64,
    estimated_output_tokens: i64,
    window_size: i64,
    now: u64,
    #[serde(default)]
    groups: BTreeMap<String, String>,
    #[serde(default)]
    cache: BTreeMap<String, CounterValueInput>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    ReserveTpmTokens(ReserveTpmInput),
    RefundReservedTokens(RefundReservedInput),
    ReserveIoTokens(IoReserveInput),
    #[serde(rename = "enforce_project_io_token_quota_for_frame")]
    Frame(FrameInput),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ReserveTpmTokens(case) => reserve_tpm(case),
            Input::RefundReservedTokens(case) => refund_reserved(case),
            Input::ReserveIoTokens(case) => reserve_io(case),
            Input::Frame(case) => enforce_frame(case),
        }
    }
}

struct Fixture {
    runtime: Runtime,
    effects: Arc<Effects>,
    adjustments: TokenAdjustments,
    descriptors: ModelDescriptorBuilder,
    clock: Arc<FixedClock>,
    providers: UnresolvedProviders,
    window_size: i64,
}

impl Fixture {
    fn new(
        window_size: i64,
        now: u64,
        groups: &BTreeMap<String, String>,
        cache: &BTreeMap<String, CounterValueInput>,
    ) -> Result<Self, Box<HelperError>> {
        let effects = Arc::new(Effects::default());
        let recording = Arc::new(RecordingCache::new(cache, Arc::clone(&effects)));
        let usage = Arc::new(RecordingUsage {
            effects: Arc::clone(&effects),
        });
        let clock = FixedClock::at(now)
            .ok_or("the case time is too large")
            .or_value_error()?;
        let adjustments = TokenAdjustments {
            windows: WindowStore {
                cache: recording,
                rate_limiter_script: None,
                counter_read_script: None,
                request_batch: None,
                check_and_increment_lock: Mutex::new(()),
            },
            usage,
            post_call_batch: None,
            token_increment_script: None,
            guarded_token_script: None,
        };
        Ok(Self {
            runtime: Builder::new_current_thread().build()?,
            effects,
            adjustments,
            descriptors: ModelDescriptorBuilder {
                window_size,
                resolver: Box::new(GroupMap {
                    groups: groups.clone(),
                }),
            },
            clock,
            providers: UnresolvedProviders,
            window_size,
        })
    }

    fn reservations(&self) -> TokenReservations<'_> {
        TokenReservations {
            settlement: AtomicSettlement {
                windows: &self.adjustments.windows,
                clock: self.clock.as_ref(),
                script: None,
                refund_script: None,
                fail_closed: FailClosedPolicy { enabled: false },
                default_window_size: self.window_size,
            },
            adjustments: &self.adjustments,
            descriptors: &self.descriptors,
            attribution: RateLimitErrorAttribution {
                requested_model: None,
                native: &self.providers,
                router: None,
            },
        }
    }
}

fn answer(
    response: Option<&RateLimitResponse>,
    itpm_reserved: i64,
    otpm_reserved: i64,
    raised: Value,
    effects: &Effects,
) -> Value {
    let rendered = match response {
        Some(rate_limit_response) => response_json(rate_limit_response),
        None => Value::Null,
    };
    json!({
        "response": rendered,
        "itpm_reserved": itpm_reserved,
        "otpm_reserved": otpm_reserved,
        "raised": raised,
        "effects": effects_json(effects),
    })
}

fn reserve_tpm(case: &ReserveTpmInput) -> HelperResult {
    let fixture = Fixture::new(case.window_size, case.now, &case.groups, &case.cache)?;
    let reservations = fixture.reservations();
    let response = fixture
        .runtime
        .block_on(reservations.reserve_tpm_tokens(&case.descriptors, case.estimated_tokens))
        .or_value_error()?;
    Ok(answer(Some(&response), 0, 0, Value::Null, &fixture.effects))
}

fn refund_reserved(case: &RefundReservedInput) -> HelperResult {
    let fixture = Fixture::new(case.window_size, case.now, &case.groups, &case.cache)?;
    let windows: BTreeSet<ReservationWindowIdentity> =
        case.reservation_windows.iter().cloned().collect();
    let reservations = fixture.reservations();
    fixture
        .runtime
        .block_on(reservations.refund_reserved_tokens(&case.scopes, case.amount, &windows))
        .or_value_error()?;
    Ok(answer(None, 0, 0, Value::Null, &fixture.effects))
}

fn reserve_io(case: &IoReserveInput) -> HelperResult {
    let fixture = Fixture::new(case.window_size, case.now, &case.groups, &case.cache)?;
    let reservations = fixture.reservations();
    let reservation = fixture
        .runtime
        .block_on(reservations.reserve_io_tokens(
            &case.descriptors,
            case.estimated_input_tokens,
            case.estimated_output_tokens,
        ))
        .or_value_error()?;
    let IoReservation {
        response,
        itpm_reserved,
        otpm_reserved,
    } = reservation;
    Ok(answer(
        Some(&response),
        itpm_reserved,
        otpm_reserved,
        Value::Null,
        &fixture.effects,
    ))
}

fn enforce_frame(case: &FrameInput) -> HelperResult {
    let fixture = Fixture::new(case.window_size, case.now, &case.groups, &case.cache)?;
    let auth = case
        .auth
        .clone()
        .map(|fields| UserAPIKeyAuth::construct(AuthInput::Dict(fields)))
        .transpose()
        .or_value_error()?;
    let result = fixture
        .runtime
        .block_on(fixture.reservations().enforce_frame_project_io_quota(
            auth.as_ref(),
            case.requested_model.as_deref(),
            case.estimated_input_tokens,
            case.estimated_output_tokens,
        ));
    let raised = match result {
        Ok(()) => Value::Null,
        Err(error) => raised_json(&error)?,
    };
    Ok(answer(None, 0, 0, raised, &fixture.effects))
}
