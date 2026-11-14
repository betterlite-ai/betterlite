use crate::helpers::{Helper, HelperError, HelperResult};
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use betterlite_server::scim_projections::group::project_group;
use betterlite_server::scim_projections::user::{ScimUserSource, project_user};
use betterlite_server::scim_records::user::User;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use std::fmt::Display;

pub(crate) struct Op;

#[derive(Deserialize)]
pub(crate) struct Input {
    kind: Kind,
    value: Box<RawValue>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    User,
    Group,
}

#[derive(Serialize)]
struct UserOutput {
    user: User,
    warnings: Vec<String>,
}

fn failure<E>(error: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "ValidationError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn user(value: &str) -> HelperResult {
    let stored = serde_json::from_str::<LiteLLMUserTable>(value).map_err(failure)?;
    let projection = project_user(&ScimUserSource::from(&stored), Vec::new()).map_err(failure)?;
    let warnings = projection
        .dropped
        .iter()
        .map(|key| {
            format!(
                "Skipping malformed {key} metadata on user {} in SCIM response",
                stored.user_id
            )
        })
        .collect();
    serde_json::to_value(UserOutput {
        user: projection.user,
        warnings,
    })
    .map_err(failure)
}

fn group(value: &str) -> HelperResult {
    let stored = serde_json::from_str::<LiteLLMTeamTable>(value).map_err(failure)?;
    let projected = project_group(&stored).map_err(failure)?;
    serde_json::to_value(projected).map_err(failure)
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        match input.kind {
            Kind::User => user(input.value.get()),
            Kind::Group => group(input.value.get()),
        }
    }
}
