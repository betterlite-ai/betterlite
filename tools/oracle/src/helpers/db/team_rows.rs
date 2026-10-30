use betterlite_db::{
    budget_permission_rows::JsonData,
    team_rows::{
        decoder::{TeamRow, TeamRowError, decode_team_rows},
        team::TeamBase,
        validator::set_model_info,
    },
};
use serde::{Deserialize, Serialize};

pub(crate) struct Op;
#[derive(Deserialize)]
pub(crate) struct Input {
    calls: Vec<Call>,
}
#[derive(Deserialize, Serialize)]
struct Call {
    name: String,
    operation: Operation,
    row: JsonData,
}
#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    Validator,
    BaseModel,
    Limits,
    MemberBase,
    Member,
    BudgetLimit,
    Model,
    TeamBase,
    Team,
    Cached,
    Deleted,
    Membership,
}
#[derive(Debug, thiserror::Error)]
enum AdapterError {
    #[error("{0}")]
    Row(#[from] TeamRowError),
    #[error("{0}")]
    Json(#[from] serde_json::Error),
    #[error("amount cannot project to float")]
    FloatProjection,
}
#[derive(Serialize)]
struct Envelope<'row> {
    kind: Operation,
    row: &'row JsonData,
}
#[derive(Serialize)]
struct Validated {
    row: JsonData,
    error: Option<String>,
}

impl crate::helpers::Helper for Op {
    type Input = Input;
    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let outputs = input.calls.iter().map(run_call).collect::<Vec<_>>();
        serde_json::to_value(outputs).map_err(|source| {
            Box::new(crate::helpers::HelperError {
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
fn run_call(call: &Call) -> JsonData {
    match execute(call) {
        Ok(value) => value,
        Err(source) => JsonData::String(normalize_error(&source.to_string())),
    }
}
fn execute(call: &Call) -> Result<JsonData, AdapterError> {
    let (kind, limits) = match call.operation {
        Operation::Validator => return validate(call.row.clone()),
        Operation::BaseModel => {
            let base: TeamBase = serde_json::from_str(&serde_json::to_string(&call.row)?)?;
            return validate(serde_json::from_str(&serde_json::to_string(&base)?)?);
        }
        Operation::Limits => (Operation::Membership, true),
        Operation::MemberBase
        | Operation::Member
        | Operation::BudgetLimit
        | Operation::Model
        | Operation::TeamBase
        | Operation::Team
        | Operation::Cached
        | Operation::Deleted
        | Operation::Membership => (call.operation, false),
    };
    project(&call.row, kind, limits)
}
fn validate(mut row: JsonData) -> Result<JsonData, AdapterError> {
    let error = set_model_info(&mut row)
        .err()
        .map(|source| source.to_string());
    serde_json::from_str(&serde_json::to_string(&Validated { row, error })?)
        .map_err(AdapterError::Json)
}
fn project(row: &JsonData, kind: Operation, limits: bool) -> Result<JsonData, AdapterError> {
    let bytes = serde_json::to_vec(&Envelope { kind, row })?;
    let decoded = decode_team_rows(&bytes)?;
    let output = if let TeamRow::Membership(member) = &decoded
        && limits
    {
        serde_json::to_string(&(
            member.safe_get_team_member_rpm_limit(),
            member.safe_get_team_member_tpm_limit(),
        ))
    } else {
        serde_json::to_string(&decoded)
    }?;
    let mut projection = serde_json::from_str(&output)?;
    float_amounts(&mut projection)?;
    Ok(projection)
}
fn normalize_error(message: &str) -> String {
    for expected in [
        "input needs to be a dictionary",
        "Either user id or user email must be provided",
    ] {
        if message.contains(expected) {
            return expected.to_owned();
        }
    }
    for field in [
        "metadata",
        "aliases",
        "config",
        "permissions",
        "model_max_budget",
        "model_aliases",
        "router_settings",
        "budget_limits",
    ] {
        let expected = format!("Field {field} should be a valid dictionary");
        if message.contains(&expected) {
            return expected;
        }
    }
    "invalid row".to_owned()
}

fn float_amounts(value: &mut JsonData) -> Result<(), AdapterError> {
    let JsonData::Object(fields) = value else {
        return Ok(());
    };
    // The oracle compares JSON number representations, while storage retains checked Decimal amounts.
    for key in [
        "max_budget",
        "soft_budget",
        "spend",
        "total_spend",
        "temp_budget_increase",
        "last_refreshed_at",
    ] {
        if let Some(JsonData::Number(amount)) = fields.get_mut(key) {
            *amount = amount
                .as_f64()
                .and_then(serde_json::Number::from_f64)
                .ok_or(AdapterError::FloatProjection)?;
        }
    }
    for key in ["row", "litellm_budget_table", "litellm_model_table", "team"] {
        if let Some(nested) = fields.get_mut(key) {
            float_amounts(nested)?;
        }
    }
    if let Some(JsonData::Array(windows)) = fields.get_mut("budget_limits") {
        for window in windows {
            float_amounts(window)?;
        }
    }
    Ok(())
}

mod tests;
