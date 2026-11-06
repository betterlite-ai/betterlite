use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_server::management_guards::amounts::validate_finite_spend;
use betterlite_server::management_guards::error::ManagementGuardError;
use serde::{Deserialize, Serialize};

use crate::helpers::{Helper, HelperError, HelperResult};

mod values;

use values::{
    amount, caller_id_value, duration_value, guarded, guardrail_value, into_outcome,
    metadata_value, passthrough_value, team_budget_value, update_value,
};

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
pub(super) struct Outcome {
    pub(super) ok: bool,
    pub(super) status: Option<u16>,
    pub(super) detail: Option<String>,
    pub(super) value: JsonData,
}

pub(super) enum GuardFailure {
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
        let outcomes: Vec<Outcome> = input.calls.iter().map(run_guard).collect();
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

fn run_guard(call: &Call) -> Outcome {
    match call {
        Call::ValidateFiniteSpend(input) => {
            let spend = amount(input.spend.as_ref());
            into_outcome(spend.and_then(|value| guarded(validate_finite_spend(value))))
        }
        Call::ValidateBudgetDuration(input) => into_outcome(duration_value(input)),
        Call::ValidateTeamModelMaxBudget(input) => into_outcome(team_budget_value(input)),
        Call::CallerUserIdForNonAdmin(input) => into_outcome(caller_id_value(input)),
        Call::CheckPassthroughRoutes(input) => into_outcome(passthrough_value(input)),
        Call::CheckGuardrailOptOut(input) => into_outcome(guardrail_value(input)),
        Call::SetObjectMetadataField(input) => into_outcome(Ok(metadata_value(input))),
        Call::UpdateMetadataField(input) => {
            into_outcome(update_value(&input.updated_kv, Some(&input.field_name)))
        }
        Call::UpdateMetadataFields(input) => into_outcome(update_value(&input.updated_kv, None)),
    }
}
