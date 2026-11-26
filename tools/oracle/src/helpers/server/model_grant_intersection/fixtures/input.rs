use std::collections::BTreeMap;

use betterlite_auth::auth_prefetch::rows::CachedProject;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::resource_rows::organization::OrganizationTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Function {
    Enforced,
    Collected,
    Stamped,
}

// A table or the router that fails its read, scripted by name so the case lists only what breaks.
#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Fault {
    Team,
    User,
    Project,
    Membership,
    Org,
    Serving,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Token {
    #[serde(default)]
    pub(crate) models: Vec<String>,
    #[serde(default)]
    pub(crate) team_models: Vec<String>,
    #[serde(default)]
    pub(crate) team_id: Option<String>,
    #[serde(default)]
    pub(crate) user_id: Option<String>,
    #[serde(default)]
    pub(crate) project_id: Option<String>,
    #[serde(default)]
    pub(crate) org_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Team {
    pub(crate) team_id: String,
    #[serde(default)]
    pub(crate) models: Vec<String>,
    #[serde(default)]
    pub(crate) organization_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct User {
    pub(crate) user_id: String,
    #[serde(default)]
    pub(crate) models: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Project {
    pub(crate) project_id: String,
    #[serde(default)]
    pub(crate) models: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Membership {
    pub(crate) user_id: String,
    pub(crate) team_id: String,
    #[serde(default)]
    pub(crate) allowed_models: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Org {
    pub(crate) organization_id: String,
    #[serde(default)]
    pub(crate) models: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) function: Function,
    pub(crate) database: bool,
    pub(crate) token: Token,
    #[serde(default)]
    pub(crate) models: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) membership_loaded: bool,
    #[serde(default)]
    pub(crate) stamp_before: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) team: Option<Team>,
    #[serde(default)]
    pub(crate) user: Option<User>,
    #[serde(default)]
    pub(crate) project: Option<Project>,
    #[serde(default)]
    pub(crate) membership: Option<Membership>,
    #[serde(default)]
    pub(crate) org: Option<Org>,
    #[serde(default)]
    pub(crate) registry: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) serving: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub(crate) outages: Vec<Fault>,
}

// The rows the scripted sources hand back, decoded from the case's payloads in one place.
pub(crate) struct Rows {
    pub(crate) team: Option<LiteLLMTeamTableCachedObj>,
    pub(crate) user: Option<LiteLLMUserTable>,
    pub(crate) project: Option<CachedProject>,
    pub(crate) org: Option<OrganizationTable>,
    pub(crate) membership: Option<LiteLLMTeamMembership>,
}

pub(crate) fn rows(input: &Input) -> Result<Rows, Box<HelperError>> {
    Ok(Rows {
        team: match &input.team {
            Some(team) => Some(serde_json::from_value(json!({
                "team_id": team.team_id,
                "models": team.models,
                "organization_id": team.organization_id,
                "last_refreshed_at": 1.0_f64,
            }))?),
            None => None,
        },
        user: match &input.user {
            Some(user) => Some(serde_json::from_value(json!({
                "user_id": user.user_id,
                "models": user.models,
            }))?),
            None => None,
        },
        project: match &input.project {
            Some(project) => Some(serde_json::from_value(json!({
                "project_id": project.project_id,
                "models": project.models,
                "last_refreshed_at": 1.0_f64,
            }))?),
            None => None,
        },
        org: match &input.org {
            Some(org) => Some(serde_json::from_value(json!({
                "organization_id": org.organization_id,
                "budget_id": "oracle-budget",
                "created_by": "oracle",
                "updated_by": "oracle",
                "models": org.models,
            }))?),
            None => None,
        },
        membership: match &input.membership {
            Some(membership) => Some(serde_json::from_value(membership_json(membership))?),
            None => None,
        },
    })
}

fn membership_json(membership: &Membership) -> Value {
    match &membership.allowed_models {
        Some(allowed) => json!({
            "user_id": membership.user_id,
            "team_id": membership.team_id,
            "litellm_budget_table": {"allowed_models": allowed},
        }),
        None => json!({
            "user_id": membership.user_id,
            "team_id": membership.team_id,
        }),
    }
}

pub(crate) fn auth(input: &Input) -> Result<UserAPIKeyAuth, Box<HelperError>> {
    let mut fields = BTreeMap::new();
    fields.insert("models".to_owned(), string_list(&input.token.models));
    fields.insert(
        "team_models".to_owned(),
        string_list(&input.token.team_models),
    );
    insert_text(&mut fields, "team_id", input.token.team_id.as_deref());
    insert_text(&mut fields, "user_id", input.token.user_id.as_deref());
    insert_text(&mut fields, "project_id", input.token.project_id.as_deref());
    insert_text(&mut fields, "org_id", input.token.org_id.as_deref());
    if let Some(stamp) = &input.stamp_before {
        fields.insert("matched_model_access_groups".to_owned(), string_list(stamp));
    }
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).or_value_error()
}

fn insert_text(fields: &mut BTreeMap<String, JsonData>, name: &str, value: Option<&str>) {
    if let Some(text) = value {
        fields.insert(name.to_owned(), JsonData::String(text.to_owned()));
    }
}

fn string_list(values: &[String]) -> JsonData {
    JsonData::Array(
        values
            .iter()
            .map(|value| JsonData::String(value.clone()))
            .collect(),
    )
}
