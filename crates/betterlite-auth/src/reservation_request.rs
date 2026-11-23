pub mod error;
pub mod lease;
pub mod ports;
pub mod routes;

mod tests;

use std::collections::BTreeSet;
use std::sync::Arc;

use betterlite_db::identity_rows::end_user::LiteLLMEndUserTable;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use betterlite_models::model_registration::JsonObject;
use indexmap::IndexMap;
use rust_decimal::Decimal;

use crate::auth_request_models::RequestedModels;
use crate::auth_request_models::catalog::{Collaborators, ManagedIdDecoders, ModelCatalog};
use crate::auth_request_models::model::get_model_from_request;
use crate::auth_request_models::sources::{DispatchContext, RequestSnapshot};
use crate::budget_counter::BudgetCounter;
use crate::reservation_admission::engine::{Admission, AdmissionRequest};
use crate::reservation_cost::EstimateRequest;
use crate::reservation_counters::CounterInput;
use crate::reservation_settlement::entry::ReservationEntry;
use crate::reservation_settlement::handle::{ReservationHandle, get_reserved_counter_keys};
use crate::resolved_key::auth::UserAPIKeyAuth;
use crate::route_match::classifier::is_llm_api_route;
use crate::route_match::templates::RouteValue;

use self::error::ReservationRequestError;
use self::lease::LeaseRenewal;
use self::ports::{
    ReservationAdmission, ReservationCostSource, ReservationCounterSource, ReservationSettlement,
};
use self::routes::is_unbilled_route;

// the authenticated request facts reserve_budget_for_request reads
pub struct ReservationRequest<'request> {
    pub body: &'request JsonObject,
    pub route: &'request str,
    pub catalog: Option<&'request dyn ModelCatalog>,
    pub decoders: &'request dyn ManagedIdDecoders,
    pub token: Option<&'request UserAPIKeyAuth>,
    pub team: Option<&'request LiteLLMTeamTable>,
    pub user: Option<&'request LiteLLMUserTable>,
    pub end_user_id: Option<&'request str>,
    pub end_user: Option<&'request LiteLLMEndUserTable>,
    pub tag_names: &'request [String],
    pub apply_user_budget_to_team_keys: bool,
    pub fail_closed_budget_enforcement: bool,
}

// the collaborators reserve_budget_for_request calls, injected so the server and the tests share one path
pub struct ReservationServices<'services> {
    pub counters: &'services dyn ReservationCounterSource,
    pub costs: &'services dyn ReservationCostSource,
    pub admission: &'services dyn ReservationAdmission,
    pub settlement: &'services dyn ReservationSettlement,
    pub renewal: Option<LeaseRenewal<'services>>,
    pub budget_exceeded_status: u16,
    pub throttle_percentage: Option<f64>,
}

// reserve_budget_for_request: the record a request holds while it runs, or none when it is not billed
pub async fn reserve_budget_for_request(
    request: &ReservationRequest<'_>,
    services: &ReservationServices<'_>,
) -> Result<Option<ReservationHandle>, ReservationRequestError> {
    let Some(token) = request.token else {
        return Ok(None);
    };
    if !is_llm_api_route(&RouteValue::Text(request.route)) {
        return Ok(None);
    }
    if is_unbilled_route(request.route) {
        return Ok(None);
    }
    if !has_request_model(request)? {
        return Ok(None);
    }
    let Some(counters) = collect_counters(request, token, services).await? else {
        return Ok(None);
    };
    let Some((input_token_counts, reservation_cost)) = estimate_costs(request, services).await?
    else {
        return Ok(None);
    };
    let Some((admission, entries)) =
        apply_reservation(request, services, &counters, reservation_cost).await?
    else {
        return Ok(None);
    };
    let handle = build_handle(
        request,
        services,
        &input_token_counts,
        admission.reserved_cost,
        entries,
    )
    .await?;
    start_renewal(&handle, services);
    Ok(Some(handle))
}

// get_model_from_request, which answers none for a request that names no model
fn has_request_model(request: &ReservationRequest<'_>) -> Result<bool, ReservationRequestError> {
    let path_params = BTreeSet::new();
    let snapshot = RequestSnapshot {
        route: request.route,
        body: request.body,
        headers: None,
        query: None,
        dispatch: DispatchContext {
            endpoint_marker: None,
            path_params: &path_params,
        },
        team_id: None,
    };
    let collaborators = Collaborators {
        catalog: request.catalog,
        decoders: request.decoders,
    };
    let model = get_model_from_request(&snapshot, &collaborators)?;
    Ok(!matches!(model, RequestedModels::None))
}

// _get_budget_counters: no counters means read-time enforcement only, before any token is counted
async fn collect_counters(
    request: &ReservationRequest<'_>,
    token: &UserAPIKeyAuth,
    services: &ReservationServices<'_>,
) -> Result<Option<Vec<BudgetCounter>>, ReservationRequestError> {
    let input = CounterInput {
        token,
        team: request.team,
        user: request.user,
        end_user_id: request.end_user_id,
        end_user: request.end_user,
        tag_names: request.tag_names,
        apply_user_budget_to_team_keys: request.apply_user_budget_to_team_keys,
    };
    let counters = services.counters.counters(&input).await?;
    Ok((!counters.is_empty()).then_some(counters))
}

// count_request_input_tokens and estimate_request_max_cost: no positive estimate leaves only read-time enforcement
async fn estimate_costs(
    request: &ReservationRequest<'_>,
    services: &ReservationServices<'_>,
) -> Result<Option<(IndexMap<String, usize>, Decimal)>, ReservationRequestError> {
    let estimate = EstimateRequest {
        body: request.body,
        route: request.route,
    };
    let counts = services.costs.input_tokens(&estimate).await?;
    let Some(reservation_cost) = services.costs.max_cost(&estimate, Some(&counts)).await? else {
        return Ok(None);
    };
    Ok((reservation_cost > Decimal::ZERO).then_some((counts, reservation_cost)))
}

// the batch scope around counter initialization and reservation, then the best-effort release of what it applied
async fn apply_reservation(
    request: &ReservationRequest<'_>,
    services: &ReservationServices<'_>,
    counters: &[BudgetCounter],
    reservation_cost: Decimal,
) -> Result<Option<(Admission, Vec<ReservationEntry>)>, ReservationRequestError> {
    let admission_request = AdmissionRequest {
        counters,
        token: request.token,
        reservation_cost,
        fail_closed_budget_enforcement: request.fail_closed_budget_enforcement,
        budget_exceeded_status: services.budget_exceeded_status,
        throttle_percentage: services.throttle_percentage,
    };
    let counter_keys: BTreeSet<String> = counters
        .iter()
        .map(|counter| counter.counter_key.clone())
        .collect();
    let mut applied_entries = Vec::new();
    let scope = services.settlement.enter_scope(&counter_keys);
    let outcome = services
        .admission
        .admit(&admission_request, &mut applied_entries)
        .await;
    drop(scope);
    let admission = match outcome {
        Ok(admission) => admission,
        Err(error) => {
            services
                .settlement
                .release_entries(&mut applied_entries, reservation_cost)
                .await;
            return Err(ReservationRequestError::Admission(error));
        }
    };
    if applied_entries.is_empty() {
        return Ok(None);
    }
    Ok(Some((admission, applied_entries)))
}

// the record every settlement path works on, its input cost capped at what the reservation actually holds
async fn build_handle(
    request: &ReservationRequest<'_>,
    services: &ReservationServices<'_>,
    input_token_counts: &IndexMap<String, usize>,
    reserved_cost: Decimal,
    entries: Vec<ReservationEntry>,
) -> Result<ReservationHandle, ReservationRequestError> {
    let estimate = EstimateRequest {
        body: request.body,
        route: request.route,
    };
    let input_cost = services
        .costs
        .input_cost(&estimate, Some(input_token_counts))
        .await?
        .unwrap_or(Decimal::ZERO)
        .min(reserved_cost);
    Ok(ReservationHandle::new(reserved_cost, entries, input_cost))
}

// _start_reservation_lease_renewal, which does nothing without a Redis backend or a reserved counter key
fn start_renewal(handle: &ReservationHandle, services: &ReservationServices<'_>) {
    let Some(renewal) = &services.renewal else {
        return;
    };
    let counter_keys = get_reserved_counter_keys(handle);
    if counter_keys.is_empty() {
        return;
    }
    lease::start(renewal, Arc::downgrade(&handle.0), counter_keys);
}
