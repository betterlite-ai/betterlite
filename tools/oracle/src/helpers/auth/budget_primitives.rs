use betterlite_auth::budget_primitives::carried::{
    carried_metadata, carry_organization, carry_team_and_user,
};
use betterlite_auth::budget_primitives::throttle::{percentage, should_throttle, throttled_limit};
use betterlite_auth::budget_primitives::window::{
    BudgetWindow, BudgetWindowModel, coerce_model, coerce_window,
};
use betterlite_auth::budget_primitives::window_start::{DurationParser, budget_window_start};
use betterlite_auth::budget_snapshots::BudgetSnapshotError;
use betterlite_auth::resolved_key::auth::{AuthError, AuthInput, UserAPIKeyAuth};
use betterlite_core::duration::seconds::duration_in_seconds;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::resource_rows::organization::OrganizationTable;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use chrono::{DateTime, Datelike as _, FixedOffset, Timelike as _, Utc};
use jiff::tz::TimeZoneDatabase;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum Call {
    ThrottlePercentage { value: PercentageValue },
    ShouldThrottle { value: ThrottleValue },
    ThrottledLimit { value: LimitValue },
    CoerceWindow { value: WindowValue },
    WindowStart { value: StartValue },
    CarryTeamUser { value: CarryValue },
    CarryOrganization { value: OrganizationValue },
    CarriedMetadata { value: MetadataValue },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PercentageValue {
    percentage: JsonData,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThrottleValue {
    token: BTreeMap<String, JsonData>,
    percentage: JsonData,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LimitValue {
    #[serde(default)]
    limit: Option<i64>,
    #[serde(default)]
    percentage: Option<f64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WindowValue {
    window: WindowShape,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StartValue {
    window: WindowShape,
    now: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CarryValue {
    token: BTreeMap<String, JsonData>,
    #[serde(default)]
    team: Option<JsonData>,
    #[serde(default)]
    user: Option<JsonData>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OrganizationValue {
    token: BTreeMap<String, JsonData>,
    organization: JsonData,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MetadataValue {
    token: BTreeMap<String, JsonData>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum WindowShape {
    Value(JsonData),
    Model(JsonData),
    RaisingModel,
}

struct FixtureDump(JsonData);

struct FixtureFailure;

#[derive(Debug, Error)]
#[error("model dump failed")]
struct DumpError;

impl BudgetWindowModel for FixtureDump {
    type Error = DumpError;

    fn model_dump(&self) -> Result<JsonData, Self::Error> {
        Ok(self.0.clone())
    }
}

impl BudgetWindowModel for FixtureFailure {
    type Error = DumpError;

    fn model_dump(&self) -> Result<JsonData, Self::Error> {
        Err(DumpError)
    }
}

struct CoreDurations(TimeZoneDatabase);

impl DurationParser for CoreDurations {
    fn seconds(&self, spec: &str, now: DateTime<Utc>) -> Option<i64> {
        duration_in_seconds(spec, now, None, &self.0)
            .ok()
            .map(|duration| duration.seconds)
    }
}

#[derive(Debug, Error)]
enum Failure {
    #[error("the token fields are invalid")]
    Token(#[source] AuthError),
    #[error("the row is invalid")]
    Row(#[source] serde_json::Error),
    #[error("the clock is invalid")]
    Clock(#[source] chrono::ParseError),
    #[error("the budget snapshot projection failed")]
    Snapshot(#[source] BudgetSnapshotError),
    #[error("the model dump failed")]
    Dump(#[from] DumpError),
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let outcomes = input
            .calls
            .iter()
            .map(|call| match execute(call) {
                Ok(value) => json!({"ok": true, "value": value}),
                Err(_failure) => json!({"ok": false}),
            })
            .collect::<Vec<Value>>();
        Ok(Value::Array(outcomes))
    }
}

fn execute(call: &Call) -> Result<Value, Failure> {
    match call {
        Call::ThrottlePercentage { value } => Ok(json!({
            "percentage": percentage(configured(&value.percentage)),
        })),
        Call::ShouldThrottle { value } => Ok(json!({
            "throttle": should_throttle(
                &UserAPIKeyAuth::construct(AuthInput::Dict(value.token.clone()))
                    .map_err(Failure::Token)?,
                configured(&value.percentage),
            ),
        })),
        Call::ThrottledLimit { value } => Ok(json!({
            "limit": throttled_limit(value.limit, value.percentage),
        })),
        Call::CoerceWindow { value } => Ok(json!({
            "window": window_of(&value.window)?,
        })),
        Call::WindowStart { value } => window_start(value),
        Call::CarryTeamUser { value } => {
            let mut carried = UserAPIKeyAuth::construct(AuthInput::Dict(value.token.clone()))
                .map_err(Failure::Token)?;
            let team = read_row::<LiteLLMTeamTable>(value.team.as_ref())?;
            let user = read_row::<LiteLLMUserTable>(value.user.as_ref())?;
            carry_team_and_user(&mut carried, team.as_ref(), user.as_ref());
            state(&carried)
        }
        Call::CarryOrganization { value } => {
            let mut carried = UserAPIKeyAuth::construct(AuthInput::Dict(value.token.clone()))
                .map_err(Failure::Token)?;
            let converted = serde_json::to_value(&value.organization).map_err(Failure::Row)?;
            let organization: OrganizationTable =
                serde_json::from_value(converted).map_err(Failure::Row)?;
            carry_organization(&mut carried, &organization);
            state(&carried)
        }
        Call::CarriedMetadata { value } => state(
            &UserAPIKeyAuth::construct(AuthInput::Dict(value.token.clone()))
                .map_err(Failure::Token)?,
        ),
    }
}

fn configured(value: &JsonData) -> Option<f64> {
    match value {
        JsonData::Number(number) => number.as_f64(),
        JsonData::Null
        | JsonData::Bool(_)
        | JsonData::String(_)
        | JsonData::Array(_)
        | JsonData::Object(_) => None,
    }
}

fn read_row<T>(value: Option<&JsonData>) -> Result<Option<T>, Failure>
where
    T: serde::de::DeserializeOwned,
{
    let Some(row) = value else {
        return Ok(None);
    };
    let converted = serde_json::to_value(row).map_err(Failure::Row)?;
    serde_json::from_value(converted)
        .map(Some)
        .map_err(Failure::Row)
}

fn window_of(shape: &WindowShape) -> Result<BudgetWindow, Failure> {
    match shape {
        WindowShape::Value(value) => Ok(coerce_window(value)),
        WindowShape::Model(value) => Ok(coerce_model(&FixtureDump(value.clone()))?),
        WindowShape::RaisingModel => Ok(coerce_model(&FixtureFailure)?),
    }
}

fn window_start(value: &StartValue) -> Result<Value, Failure> {
    let window = window_of(&value.window)?;
    let now = DateTime::parse_from_rfc3339(&value.now)
        .map_err(Failure::Clock)?
        .with_timezone(&Utc);
    let durations = CoreDurations(TimeZoneDatabase::from_env());
    let start = budget_window_start(&window, now, &durations);
    Ok(json!({
        "start": start.as_ref().map(isoformat),
        "offset_seconds": start.as_ref().map(|instant| instant.offset().local_minus_utc()),
    }))
}

fn state(token: &UserAPIKeyAuth) -> Result<Value, Failure> {
    let metadata = carried_metadata(token).map_err(Failure::Snapshot)?;
    let serialized = serde_json::to_value(token).map_err(Failure::Row)?;
    Ok(json!({
        "metadata": metadata,
        "team_model_max_budget": serialized.get("team_model_max_budget").cloned().unwrap_or(Value::Null),
        "organization_alias": serialized.get("organization_alias").cloned().unwrap_or(Value::Null),
    }))
}

fn isoformat(value: &DateTime<FixedOffset>) -> String {
    let local = value.naive_local();
    let base = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        local.year(),
        local.month(),
        local.day(),
        local.hour(),
        local.minute(),
        local.second()
    );
    let micro = local.and_utc().timestamp_subsec_micros();
    if micro == 0 {
        base
    } else {
        format!("{base}.{micro:06}")
    }
}
