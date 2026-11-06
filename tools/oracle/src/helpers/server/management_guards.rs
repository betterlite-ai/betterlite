use std::collections::BTreeMap;

use axum::http::StatusCode;
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey, UserIdentity,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType, map_role};
use betterlite_config::sections::budget_reset::BudgetResetSettings;
use betterlite_core::budget_reset::ResetClocks;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_server::management_guards::amounts::{Amount, validate_finite_spend};
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
use serde::{Deserialize, Serialize};

use crate::helpers::{Helper, HelperError, HelperResult};

const DATABASE: fn() -> TimeZoneDatabase = TimeZoneDatabase::from_env;

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum Call {
    ValidateFiniteSpend(SpendCall),
    ValidateBudgetDuration(DurationCall),
    ValidateTeamModelMaxBudget(TeamBudgetCall),
    CallerUserIdForNonAdmin(CallerCall),
    CheckPassthroughRoutes(PassthroughCall),
    CheckGuardrailOptOut(GuardrailCall),
    SetObjectMetadataField(SetMetadataCall),
    UpdateMetadataField(UpdateFieldCall),
    UpdateMetadataFields(UpdateFieldsCall),
}

#[derive(Debug, Deserialize)]
struct SpendCall {
    #[serde(default)]
    spend: Option<JsonData>,
}

#[derive(Debug, Deserialize)]
struct DurationCall {
    #[serde(default)]
    budget_duration: Option<String>,
    #[serde(default)]
    status_code: Option<u16>,
}

#[derive(Debug, Deserialize)]
struct TeamBudgetCall {
    #[serde(default)]
    model_max_budget: Option<BTreeMap<String, BudgetInput>>,
}

#[derive(Debug, Deserialize)]
struct BudgetInput {
    #[serde(default)]
    max_budget: Option<JsonData>,
    #[serde(default)]
    budget_duration: Option<String>,
    #[serde(default)]
    tpm_limit: Option<i64>,
    #[serde(default)]
    rpm_limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct CallerCall {
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    user_role: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PassthroughCall {
    #[serde(default)]
    user_role: Option<String>,
    #[serde(default)]
    allowed_passthrough_routes: Option<Vec<JsonData>>,
    #[serde(default)]
    metadata: Option<JsonData>,
    #[serde(default = "default_entity")]
    entity: String,
}

#[derive(Debug, Deserialize)]
struct GuardrailCall {
    #[serde(default)]
    user_role: Option<String>,
    #[serde(default)]
    disable_global_guardrails: Option<bool>,
    #[serde(default)]
    metadata: Option<JsonData>,
    #[serde(default)]
    existing_metadata: Option<JsonData>,
    #[serde(default = "default_entity")]
    entity: String,
}

#[derive(Debug, Deserialize)]
struct SetMetadataCall {
    #[serde(default)]
    metadata: Option<BTreeMap<String, JsonData>>,
    field_name: String,
    value: JsonData,
}

#[derive(Debug, Deserialize)]
struct UpdateFieldCall {
    updated_kv: BTreeMap<String, JsonData>,
    field_name: String,
}

#[derive(Debug, Deserialize)]
struct UpdateFieldsCall {
    updated_kv: BTreeMap<String, JsonData>,
}

#[derive(Debug, Serialize)]
struct Outcome {
    ok: bool,
    status: Option<u16>,
    detail: Option<String>,
    value: JsonData,
}

enum GuardFailure {
    Rejected {
        error: ManagementGuardError,
        value: JsonData,
    },
    InvalidInput(&'static str),
}

impl From<ManagementGuardError> for GuardFailure {
    fn from(error: ManagementGuardError) -> Self {
        Self::Rejected {
            error,
            value: JsonData::Null,
        }
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let outcomes = input
            .calls
            .iter()
            .map(run_call)
            .collect::<Result<Vec<_>, Box<HelperError>>>()?;
        serde_json::to_value(outcomes).map_err(|source| {
            Box::new(HelperError {
                class: "Output".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: source.to_string(),
            })
        })
    }
}

fn default_entity() -> String {
    "key".to_owned()
}

fn run_call(call: &Call) -> Result<Outcome, Box<HelperError>> {
    match run_guard(call) {
        Ok(value) => Ok(Outcome {
            ok: true,
            status: None,
            detail: None,
            value,
        }),
        Err(GuardFailure::Rejected { error, value }) => Ok(Outcome {
            ok: false,
            status: error.status_code().map(|status| status.as_u16()),
            detail: Some(error.to_string()),
            value,
        }),
        Err(GuardFailure::InvalidInput(message)) => Err(Box::new(HelperError {
            class: "InvalidInput".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: message.to_owned(),
        })),
    }
}

fn run_guard(call: &Call) -> Result<JsonData, GuardFailure> {
    match call {
        Call::ValidateFiniteSpend(input) => spend_value(input),
        Call::ValidateBudgetDuration(input) => duration_value(input),
        Call::ValidateTeamModelMaxBudget(input) => team_budget_value(input),
        Call::CallerUserIdForNonAdmin(input) => caller_id_value(input),
        Call::CheckPassthroughRoutes(input) => passthrough_value(input),
        Call::CheckGuardrailOptOut(input) => guardrail_value(input),
        Call::SetObjectMetadataField(input) => Ok(metadata_value(input)),
        Call::UpdateMetadataField(input) => update_field_value(input),
        Call::UpdateMetadataFields(input) => update_fields_value(input),
    }
}

fn spend_value(input: &SpendCall) -> Result<JsonData, GuardFailure> {
    let amount = match &input.spend {
        Some(value) => Some(amount_from_json(value).ok_or(GuardFailure::InvalidInput(
            "an amount is a number or one of nan, inf, -inf",
        ))?),
        None => None,
    };
    validate_finite_spend(amount)?;
    Ok(JsonData::Null)
}

fn duration_value(input: &DurationCall) -> Result<JsonData, GuardFailure> {
    let requested = input.status_code.unwrap_or(400);
    let status = StatusCode::from_u16(requested)
        .map_err(|_| GuardFailure::InvalidInput("status_code must be a valid HTTP status"))?;
    validate_budget_duration(
        input.budget_duration.as_deref(),
        status,
        &BudgetResetSettings::default(),
        clocks(),
        &DATABASE(),
    )?;
    Ok(JsonData::Null)
}

fn team_budget_value(input: &TeamBudgetCall) -> Result<JsonData, GuardFailure> {
    let Some(raw_configs) = &input.model_max_budget else {
        validate_team_model_max_budget(
            None,
            &BudgetResetSettings::default(),
            clocks(),
            &DATABASE(),
        )?;
        return Ok(JsonData::Null);
    };
    let configs = raw_configs
        .iter()
        .map(|(name, budget)| Ok((name.clone(), budget_config(budget)?)))
        .collect::<Result<BTreeMap<_, _>, GuardFailure>>()?;
    validate_team_model_max_budget(
        Some(&configs),
        &BudgetResetSettings::default(),
        clocks(),
        &DATABASE(),
    )?;
    Ok(JsonData::Null)
}

fn budget_config(budget: &BudgetInput) -> Result<ModelBudgetConfig<'_>, GuardFailure> {
    let max_budget = match &budget.max_budget {
        Some(value) => Some(amount_from_json(value).ok_or(GuardFailure::InvalidInput(
            "an amount is a number or one of nan, inf, -inf",
        ))?),
        None => None,
    };
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

fn caller_id_value(input: &CallerCall) -> Result<JsonData, GuardFailure> {
    let caller = principal(input.user_id.as_deref(), input.user_role.as_deref());
    let user_id = caller_user_id_for_non_admin(&caller)?;
    Ok(JsonData::String(user_id.to_owned()))
}

fn passthrough_value(input: &PassthroughCall) -> Result<JsonData, GuardFailure> {
    let caller = principal(Some("oracle-caller"), input.user_role.as_deref());
    check_passthrough_routes(
        &caller,
        input.allowed_passthrough_routes.as_deref(),
        input.metadata.as_ref(),
        &input.entity,
    )?;
    Ok(JsonData::Null)
}

fn guardrail_value(input: &GuardrailCall) -> Result<JsonData, GuardFailure> {
    let caller = principal(Some("oracle-caller"), input.user_role.as_deref());
    check_guardrail_opt_out(
        &caller,
        input.disable_global_guardrails,
        input.metadata.as_ref(),
        input.existing_metadata.as_ref(),
        &input.entity,
    )?;
    Ok(JsonData::Null)
}

fn metadata_value(input: &SetMetadataCall) -> JsonData {
    let mut applied = input.metadata.clone();
    set_object_metadata_field(&mut applied, &input.field_name, input.value.clone());
    applied.map_or(JsonData::Null, JsonData::Object)
}

fn update_field_value(input: &UpdateFieldCall) -> Result<JsonData, GuardFailure> {
    let mut updated = input.updated_kv.clone();
    match update_metadata_field(&mut updated, &input.field_name) {
        Ok(()) => Ok(JsonData::Object(updated)),
        Err(error) => Err(GuardFailure::Rejected {
            error,
            value: JsonData::Object(updated),
        }),
    }
}

fn update_fields_value(input: &UpdateFieldsCall) -> Result<JsonData, GuardFailure> {
    let mut updated = input.updated_kv.clone();
    match update_metadata_fields(&mut updated) {
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
