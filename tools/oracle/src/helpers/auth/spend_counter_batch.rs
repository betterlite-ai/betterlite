use std::collections::{BTreeMap, BTreeSet};

use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_auth::spend_counter_batch::keys::{
    CounterId, EntityCounterIds, PostCallCounterKeys, admission_counter_keys,
    post_call_counter_keys,
};
use betterlite_db::budget_permission_rows::JsonData;
use serde::Deserialize;

use crate::helpers::{HelperError, HelperResult, OrHelperError as _};

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
    token_end_user_id: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct PostCallFields {
    token: JsonData,
    team_id: JsonData,
    user_id: JsonData,
    org_id: JsonData,
    end_user_id: JsonData,
    project_id: JsonData,
    tags: Vec<JsonData>,
    model_access_groups: Vec<JsonData>,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
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
    insert_text(
        &mut values,
        "end_user_id",
        fields.token_end_user_id.as_deref(),
    );
    let auth = UserAPIKeyAuth::construct(AuthInput::Dict(values)).or_value_error()?;
    Ok(admission_counter_keys(&auth, fields.end_user_id.as_deref()))
}

fn post_call_keys(fields: &PostCallFields) -> BTreeSet<String> {
    post_call_counter_keys(&PostCallCounterKeys {
        entity: EntityCounterIds {
            token: CounterId::from_json(Some(&fields.token)),
            team_id: CounterId::from_json(Some(&fields.team_id)),
            user_id: CounterId::from_json(Some(&fields.user_id)),
            org_id: CounterId::from_json(Some(&fields.org_id)),
            project_id: CounterId::from_json(Some(&fields.project_id)),
            end_user_id: CounterId::from_json(Some(&fields.end_user_id)),
        },
        tags: &fields.tags,
        model_access_groups: &fields.model_access_groups,
    })
}

fn insert_text(values: &mut BTreeMap<String, JsonData>, name: &str, value: Option<&str>) {
    if let Some(text) = value {
        values.insert(name.to_owned(), JsonData::String(text.to_owned()));
    }
}
