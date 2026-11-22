use betterlite_auth::auth_team_lookup::error::Detail;
use betterlite_auth::identity_grants::denial::{PublicDenial, raise_public};
use betterlite_auth::identity_grants::records::GrantDenial;
use betterlite_auth::identity_grants::resolver::{canonical_user_id, user_models};
use betterlite_auth::identity_grants::team_grants::{team_grants, team_json_columns};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use serde::Deserialize;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Deserialize)]
pub(crate) struct UserRow {
    user: Option<LiteLLMUserTable>,
    user_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub(crate) enum DenialRow {
    UserGone { user_id: String },
    TeamGone { team_id: String },
    NotAMember { user_id: String, team_id: String },
}

// The team stays raw JSON so a row LiteLLM_TeamTable refuses is reported instead of failing the call.
#[derive(Deserialize)]
pub(crate) struct TeamRow {
    team: Option<JsonData>,
    membership: Option<LiteLLMTeamMembership>,
    user_id: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct Input {
    users: Vec<UserRow>,
    denials: Vec<DenialRow>,
    teams: Vec<TeamRow>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut users = Vec::with_capacity(input.users.len());
        for row in &input.users {
            users.push(json!({
                "models": row.user.as_ref().map(user_models),
                "canonical_user_id": canonical_user_id(row.user_id.as_deref(), row.user.as_ref()),
            }));
        }
        let mut denials = Vec::with_capacity(input.denials.len());
        for row in &input.denials {
            denials.push(denial_view(&raise_public(grant_denial(row))));
        }
        let mut teams = Vec::with_capacity(input.teams.len());
        for row in &input.teams {
            teams.push(team_view(row)?);
        }
        Ok(json!({ "users": users, "denials": denials, "teams": teams }))
    }
}

fn grant_denial(row: &DenialRow) -> GrantDenial {
    match row {
        DenialRow::UserGone { user_id } => GrantDenial::UserGone {
            user_id: user_id.clone(),
        },
        DenialRow::TeamGone { team_id } => GrantDenial::TeamGone {
            team_id: team_id.clone(),
        },
        DenialRow::NotAMember { user_id, team_id } => GrantDenial::NotAMember {
            user_id: user_id.clone(),
            team_id: team_id.clone(),
        },
    }
}

// The raw exception a denial raises: a ProxyException's own fields, or an HTTPException's status and detail.
fn denial_view(public: &PublicDenial) -> Value {
    let detail = public.to_string();
    match (public, public.http()) {
        (PublicDenial::UserGone(exception), Some(_) | None) => json!({
            "category": "proxy",
            "status": null,
            "error_type": exception.error_type,
            "param": exception.param,
            "code": exception.code,
            "detail": exception.message,
        }),
        (PublicDenial::TeamGone(_) | PublicDenial::NotAMember { .. }, Some((status, shape))) => {
            let body = match shape {
                Detail::Error => json!({ "error": detail }),
                Detail::Text => json!(detail),
            };
            json!({
                "category": "http",
                "status": status,
                "error_type": null,
                "param": null,
                "code": null,
                "detail": body,
            })
        }
        (PublicDenial::TeamGone(_) | PublicDenial::NotAMember { .. }, None) => {
            json!({ "category": "exception", "detail": detail })
        }
    }
}

fn team_view(row: &TeamRow) -> HelperResult {
    let Some(stored) = row.team.clone() else {
        return Ok(json!({ "row": "absent", "model_aliases": null, "grants": {} }));
    };
    let Ok(team) = LiteLLMTeamTable::try_from(stored) else {
        return Ok(json!({ "row": "invalid", "model_aliases": null, "grants": null }));
    };
    let grants = team_grants(&team, row.membership.as_ref(), row.user_id.as_deref());
    Ok(json!({
        "row": "valid",
        "model_aliases": team_json_columns(&team).model_aliases,
        "grants": serde_json::to_value(&grants).or_value_error()?,
    }))
}
