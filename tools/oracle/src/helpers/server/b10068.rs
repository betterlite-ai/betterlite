use std::collections::BTreeMap;

use betterlite_auth::auth_team_lookup::lookup::TeamLookup;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_auth::user_roles::UserRole;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::key::LiteLLMVerificationToken;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::key_info::access::can_user_query_key_info;
use serde::{Deserialize, Serialize};
use tokio::runtime::{Builder, Runtime};

mod fakes;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    caller: CallerInput,
    #[serde(default)]
    key: Option<String>,
    key_row: KeyRow,
}

// the UserAPIKeyAuth fields a case sets; construct hashes an sk- api_key on both sides
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CallerInput {
    #[serde(default)]
    user_role: Option<UserRole>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    user_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyRow {
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    team_id: Option<String>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread().build()?;
        let teams = fakes::team_lookup();
        let answers = input
            .calls
            .iter()
            .map(|call| answer(&runtime, call, &teams))
            .collect::<Result<Vec<bool>, Box<HelperError>>>()?;
        Ok(serde_json::to_value(answers)?)
    }
}

fn answer(runtime: &Runtime, call: &Call, teams: &TeamLookup) -> Result<bool, Box<HelperError>> {
    let fields: BTreeMap<String, JsonData> =
        serde_json::from_value(serde_json::to_value(&call.caller)?)?;
    let auth = UserAPIKeyAuth::construct(AuthInput::Dict(fields)).or_value_error()?;
    let row = serde_json::json!({
        "user_id": &call.key_row.user_id,
        "team_id": &call.key_row.team_id,
    });
    let key_row: LiteLLMVerificationToken = serde_json::from_value(row).or_value_error()?;
    runtime
        .block_on(can_user_query_key_info(
            &auth,
            call.key.as_deref(),
            &key_row,
            teams,
        ))
        .or_value_error()
}
