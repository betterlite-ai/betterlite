use std::collections::BTreeMap;

use axum::http::StatusCode;
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey, UserIdentity,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType, map_role};
use betterlite_config::sections::budget_reset::BudgetResetSettings;
use betterlite_core::budget_reset::ResetClocks;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_server::management_guards::amounts::Amount;
use betterlite_server::management_guards::caller::{
    caller_user_id_for_non_admin, check_guardrail_opt_out, check_passthrough_routes,
};
use betterlite_server::management_guards::duration::validate_budget_duration;
use betterlite_server::management_guards::error::ManagementGuardError;
use betterlite_server::management_guards::metadata::{
    set_object_metadata_field, update_metadata_field, update_metadata_fields,
};
use betterlite_server::management_guards::team_budget::{
    ModelBudgetConfig, validate_team_model_max_budget,
};
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;

use super::{
    BudgetInput, CallerCall, DurationCall, GuardFailure, GuardrailCall, Outcome, PassthroughCall,
    SetMetadataCall, TeamBudgetCall,
};

const CALLER: &str = "oracle-caller";
const DATABASE: fn() -> TimeZoneDatabase = TimeZoneDatabase::from_env;
const INVALID_AMOUNT: GuardFailure =
    GuardFailure::InvalidInput("an amount is a number or one of nan, inf, -inf");

pub(super) fn into_outcome(result: Result<JsonData, GuardFailure>) -> Outcome {
    match result {
        Ok(value) => Outcome {
            ok: true,
            status: None,
            detail: None,
            value,
        },
        Err(GuardFailure::Rejected { error, value }) => Outcome {
            ok: false,
            status: error.status_code().map(|status| status.as_u16()),
            detail: Some(error.to_string()),
            value,
        },
        Err(GuardFailure::InvalidInput(message)) => Outcome {
            ok: false,
            status: None,
            detail: Some(message.to_owned()),
            value: JsonData::Null,
        },
    }
}

pub(super) fn guarded(result: Result<(), ManagementGuardError>) -> Result<JsonData, GuardFailure> {
    result.map_err(GuardFailure::from)?;
    Ok(JsonData::Null)
}

pub(super) fn amount(value: Option<&JsonData>) -> Result<Option<Amount>, GuardFailure> {
    value
        .map(|json| amount_from_json(json).ok_or(INVALID_AMOUNT))
        .transpose()
}

pub(super) fn duration_value(input: &DurationCall) -> Result<JsonData, GuardFailure> {
    let status = http_status(input.status_code)?;
    let (settings, clocks) = (BudgetResetSettings::default(), clocks());
    let duration = input.budget_duration.as_deref();
    guarded(validate_budget_duration(
        duration,
        status,
        &settings,
        clocks,
        &DATABASE(),
    ))
}

fn http_status(code: Option<u16>) -> Result<StatusCode, GuardFailure> {
    StatusCode::from_u16(code.unwrap_or(400))
        .map_err(|_| GuardFailure::InvalidInput("status_code must be a valid HTTP status"))
}

pub(super) fn team_budget_value(input: &TeamBudgetCall) -> Result<JsonData, GuardFailure> {
    let configs = budget_configs(input.model_max_budget.as_ref())?;
    let (settings, clocks) = (BudgetResetSettings::default(), clocks());
    guarded(validate_team_model_max_budget(
        Some(&configs),
        &settings,
        clocks,
        &DATABASE(),
    ))
}

fn budget_configs(
    raw: Option<&BTreeMap<String, BudgetInput>>,
) -> Result<BTreeMap<String, ModelBudgetConfig<'_>>, GuardFailure> {
    let Some(configs) = raw else {
        return Ok(BTreeMap::new());
    };
    configs
        .iter()
        .map(|(name, budget)| Ok((name.clone(), budget_config(budget)?)))
        .collect()
}

fn budget_config(budget: &BudgetInput) -> Result<ModelBudgetConfig<'_>, GuardFailure> {
    let max_budget = amount(budget.max_budget.as_ref())?;
    Ok(ModelBudgetConfig {
        max_budget,
        budget_duration: budget.budget_duration.as_deref(),
        tpm_limit: budget.tpm_limit,
        rpm_limit: budget.rpm_limit,
    })
}

fn amount_from_json(value: &JsonData) -> Option<Amount> {
    match value {
        JsonData::Number(number) => number
            .to_string()
            .parse::<Decimal>()
            .ok()
            .map(Amount::Finite),
        JsonData::String(text) => match text.as_str() {
            "nan" => Some(Amount::Nan),
            "inf" | "infinity" => Some(Amount::PositiveInfinity),
            "-inf" | "-infinity" => Some(Amount::NegativeInfinity),
            _ => None,
        },
        JsonData::Null | JsonData::Bool(_) | JsonData::Array(_) | JsonData::Object(_) => None,
    }
}

pub(super) fn caller_id_value(input: &CallerCall) -> Result<JsonData, GuardFailure> {
    let caller = principal(input.user_id.as_deref(), input.user_role.as_deref());
    let user_id = caller_user_id_for_non_admin(&caller)?;
    Ok(JsonData::String(user_id.to_owned()))
}

pub(super) fn passthrough_value(input: &PassthroughCall) -> Result<JsonData, GuardFailure> {
    let caller = principal(Some(CALLER), input.user_role.as_deref());
    let routes = input.allowed_passthrough_routes.as_deref();
    guarded(check_passthrough_routes(
        &caller,
        routes,
        input.metadata.as_ref(),
        &input.entity,
    ))
}

pub(super) fn guardrail_value(input: &GuardrailCall) -> Result<JsonData, GuardFailure> {
    let caller = principal(Some(CALLER), input.user_role.as_deref());
    let metadata = input.metadata.as_ref();
    let existing = input.existing_metadata.as_ref();
    let opt_out = input.disable_global_guardrails;
    guarded(check_guardrail_opt_out(
        &caller,
        opt_out,
        metadata,
        existing,
        &input.entity,
    ))
}

pub(super) fn metadata_value(input: &SetMetadataCall) -> JsonData {
    let mut applied = input.metadata.clone();
    set_object_metadata_field(&mut applied, &input.field_name, input.value.clone());
    applied.map_or(JsonData::Null, JsonData::Object)
}

pub(super) fn update_value(
    updated_kv: &BTreeMap<String, JsonData>,
    field_name: Option<&str>,
) -> Result<JsonData, GuardFailure> {
    let mut updated = updated_kv.clone();
    let outcome = match field_name {
        Some(name) => update_metadata_field(&mut updated, name),
        None => update_metadata_fields(&mut updated),
    };
    match outcome {
        Ok(()) => Ok(JsonData::Object(updated)),
        Err(error) => Err(GuardFailure::Rejected {
            error,
            value: JsonData::Object(updated),
        }),
    }
}

fn principal(user_id: Option<&str>, user_role: Option<&str>) -> Principal {
    Principal {
        principal_type: PrincipalType::Human,
        subject: user_id.unwrap_or("service-account").to_owned(),
        issuer: None,
        audience: Vec::new(),
        user: user_id.map(|id| UserIdentity {
            id: id.to_owned(),
            external_id: None,
            user_name: None,
            email: None,
            display_name: None,
        }),
        organization: None,
        teams: Vec::new(),
        project: None,
        end_user: None,
        roles: map_role(user_role).into_iter().collect(),
        scopes: Vec::new(),
        auth_method: AuthMethod::ApiKey,
        credential_ref: CredentialRef::default(),
        network: NetworkContext::default(),
        source_key: SourceKey::default(),
    }
}

const fn clocks() -> ResetClocks<'static> {
    let now: DateTime<Utc> = DateTime::UNIX_EPOCH;
    ResetClocks {
        reset_now: now,
        calendar_now: now,
        calendar_timezone: None,
    }
}
