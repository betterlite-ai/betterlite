use std::collections::{BTreeMap, BTreeSet};

use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_auth::spend_batch::keys::{
    CounterEntities, PostCallCounters, admission_counter_keys, post_call_counter_keys,
};
use betterlite_db::budget_permission_rows::JsonData;
use serde::Deserialize;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    AdmissionCounterKeys(AdmissionFields),
    PostCallCounterKeys(PostCallFields),
}

#[derive(Deserialize)]
pub(crate) struct AdmissionFields {
    token: Option<String>,
    team_id: Option<String>,
    user_id: Option<String>,
    org_id: Option<String>,
    project_id: Option<String>,
    end_user_id: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct PostCallFields {
    token: Option<String>,
    team_id: Option<String>,
    user_id: Option<String>,
    org_id: Option<String>,
    end_user_id: Option<String>,
    project_id: Option<String>,
    tags: Vec<JsonData>,
    model_access_groups: Vec<JsonData>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let keys = match input {
            Input::AdmissionCounterKeys(fields) => admission_keys(fields)?,
            Input::PostCallCounterKeys(fields) => post_call_keys(fields),
        };
        Ok(serde_json::to_value(
            keys.into_iter().collect::<Vec<String>>(),
        )?)
    }
}

fn admission_keys(fields: &AdmissionFields) -> Result<BTreeSet<String>, Box<HelperError>> {
    let mut values = BTreeMap::new();
    insert_text(&mut values, "token", fields.token.as_deref());
    insert_text(&mut values, "team_id", fields.team_id.as_deref());
    insert_text(&mut values, "user_id", fields.user_id.as_deref());
    insert_text(&mut values, "org_id", fields.org_id.as_deref());
    insert_text(&mut values, "project_id", fields.project_id.as_deref());
    let auth = UserAPIKeyAuth::construct(AuthInput::Dict(values)).or_value_error()?;
    Ok(admission_counter_keys(&auth, fields.end_user_id.as_deref()))
}

fn post_call_keys(fields: &PostCallFields) -> BTreeSet<String> {
    post_call_counter_keys(&PostCallCounters {
        entities: CounterEntities {
            key_token: fields.token.as_deref(),
            team: fields.team_id.as_deref(),
            user: fields.user_id.as_deref(),
            end_user: fields.end_user_id.as_deref(),
            org: fields.org_id.as_deref(),
            project: fields.project_id.as_deref(),
        },
        tags: &fields.tags,
        access_groups: &fields.model_access_groups,
    })
}

fn insert_text(values: &mut BTreeMap<String, JsonData>, name: &str, value: Option<&str>) {
    if let Some(text) = value {
        values.insert(name.to_owned(), JsonData::String(text.to_owned()));
    }
}
