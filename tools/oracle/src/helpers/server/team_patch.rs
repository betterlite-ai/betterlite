use std::collections::BTreeMap;
use std::error::Error;

use betterlite_config::sections::budget_reset::BudgetResetSettings;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::key_policy_fields::KeyClock;
use betterlite_server::team_patch::budget::set_budget_reset_at;
use betterlite_server::team_request_contracts::shared::RequestProjection as _;
use betterlite_server::team_request_contracts::update::UpdateTeamRequest;
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    request: JsonData,
    now: String,
}

#[derive(Debug)]
struct CallFailed;

// the oracle reports one failed outcome per call, so every error collapses into this marker
impl<E: Error> From<E> for CallFailed {
    fn from(_: E) -> Self {
        Self
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let outcomes = input
            .calls
            .iter()
            .map(|call| match answer(call) {
                Ok(updated) => json!({"ok": true, "value": updated}),
                Err(CallFailed) => json!({"ok": false}),
            })
            .collect();
        Ok(Value::Array(outcomes))
    }
}

fn answer(call: &Call) -> Result<Value, CallFailed> {
    let request = UpdateTeamRequest::from_value(&call.request)?;
    let JsonData::Object(mut updated_kv) = request.supplied()? else {
        return Err(CallFailed);
    };
    let reset = BudgetResetSettings::default();
    let database = TimeZoneDatabase::from_env();
    // the oracle always runs the default UTC reset policy; the case freezes the request time
    let now = DateTime::parse_from_rfc3339(&call.now)?.with_timezone(&Utc);
    let clock = KeyClock {
        now,
        reset: &reset,
        database: &database,
    };
    set_budget_reset_at(&request.fields, &mut updated_kv, clock)?;
    parsed_windows(&mut updated_kv)?;
    Ok(serde_json::to_value(&updated_kv)?)
}

// LiteLLM's json.dumps spacing is not the contract, so both sides report the windows as parsed JSON
fn parsed_windows(updated_kv: &mut BTreeMap<String, JsonData>) -> Result<(), CallFailed> {
    if let Some(JsonData::String(text)) = updated_kv.get("budget_limits").cloned() {
        let parsed: JsonData = serde_json::from_str(&text)?;
        updated_kv.insert("budget_limits".to_owned(), parsed);
    }
    Ok(())
}
