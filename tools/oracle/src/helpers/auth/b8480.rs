mod support;

use std::collections::BTreeMap;

use betterlite_auth::reservation_counters::CounterServices;
use betterlite_auth::reservation_request::production::RequestCounters;
use betterlite_auth::reservation_request::{
    ReservationRequest, ReservationServices, reserve_budget_for_request,
};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::model_registration::JsonObject;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_redis::request_batch::lock;
use chrono::DateTime;
use jiff::tz::TimeZoneDatabase;
use serde::Deserialize;
use serde_json::{Number, json};
use tokio::runtime::Builder;

use self::support::{
    Durations, NeverAdmission, NeverSettlement, NoCache, NoDecoders, NoGroups, NoTags,
    RecordingCosts,
};

// litellm.budget_exceeded_status_code's default
const BUDGET_EXCEEDED_STATUS: u16 = 422;
// the counter builder needs a clock its window start can subtract a day from
const NOW_SECONDS: i64 = 1_704_067_200;
const MAX_BUDGET: u64 = 100;
const REQUEST_BODY: &str = r#"{"model":"gpt-4o"}"#;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    routes: Vec<String>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_time()
            .build()
            .map_err(|error| invalid(error.to_string()))?;
        runtime.block_on(execute(input))
    }
}

// reserve_budget_for_request's route filters: whether a route reached token counting is the observable
async fn execute(input: &Input) -> HelperResult {
    let now = DateTime::from_timestamp(NOW_SECONDS, 0)
        .ok_or_else(|| invalid("the fixed clock is outside the supported range".to_owned()))?;
    let cache = NoCache;
    let tags = NoTags;
    let groups = NoGroups;
    let durations = Durations(TimeZoneDatabase::from_env());
    let counters = RequestCounters {
        services: CounterServices {
            database: false,
            cache: &cache,
            tags: &tags,
            groups: &groups,
            durations: &durations,
            now,
        },
    };
    let costs = RecordingCosts::default();
    let admission = NeverAdmission;
    let settlement = NeverSettlement;
    let services = build_services(&counters, &costs, &admission, &settlement);
    let body: JsonObject =
        serde_json::from_str(REQUEST_BODY).map_err(|error| invalid(error.to_string()))?;
    let token = token()?;
    let decoders = NoDecoders;
    let mut counted = Vec::with_capacity(input.routes.len());
    for route in &input.routes {
        *lock(&costs.counted) = false;
        let request = build_request(&body, route, &token, &decoders);
        reserve_budget_for_request(&request, &services)
            .await
            .map_err(|error| invalid(error.to_string()))?;
        counted.push(*lock(&costs.counted));
    }
    Ok(json!({"counted": counted}))
}

fn build_services<'services>(
    counters: &'services RequestCounters<'services>,
    costs: &'services RecordingCosts,
    admission: &'services NeverAdmission,
    settlement: &'services NeverSettlement,
) -> ReservationServices<'services> {
    ReservationServices {
        counters,
        costs,
        admission,
        settlement,
        renewal: None,
        budget_exceeded_status: BUDGET_EXCEEDED_STATUS,
        throttle_percentage: None,
    }
}

fn build_request<'request>(
    body: &'request JsonObject,
    route: &'request str,
    token: &'request UserAPIKeyAuth,
    decoders: &'request NoDecoders,
) -> ReservationRequest<'request> {
    ReservationRequest {
        body,
        route,
        catalog: None,
        decoders,
        token: Some(token),
        team: None,
        user: None,
        end_user_id: None,
        end_user: None,
        tag_names: &[],
        apply_user_budget_to_team_keys: false,
        fail_closed_budget_enforcement: false,
    }
}

fn token() -> Result<UserAPIKeyAuth, Box<HelperError>> {
    let fields = BTreeMap::from([
        ("token".to_owned(), JsonData::String("case-key".to_owned())),
        (
            "max_budget".to_owned(),
            JsonData::Number(Number::from(MAX_BUDGET)),
        ),
        ("spend".to_owned(), JsonData::Number(Number::from(0_u64))),
    ]);
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).map_err(|error| invalid(error.to_string()))
}

fn invalid(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
