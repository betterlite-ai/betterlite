use crate::helpers::{Helper, HelperError, HelperResult};
use betterlite_config::scim_role_settings::{ScimUserRole, Settings};
use betterlite_server::scim_projections::directory::Metadata;
use betterlite_server::scim_records::user::User;
use betterlite_server::scim_role_settings::data::extract_scim_user_data;
use betterlite_server::scim_role_settings::metadata::build_scim_metadata;
use betterlite_server::scim_role_settings::roles::{
    default_scim_user_role, resolve_scim_user_role,
};
use serde::{Deserialize, Serialize};
use std::fmt::Display;

pub(crate) struct Op;

#[derive(Deserialize)]
pub(crate) struct Input {
    user: User,
    #[serde(default)]
    admin_group: Option<String>,
    #[serde(default)]
    default_role: Option<ScimUserRole>,
    #[serde(default)]
    active: Option<bool>,
}

#[derive(Serialize)]
struct Output {
    user_email: Option<String>,
    user_alias: Option<String>,
    sso_user_id: Option<String>,
    teams: Vec<String>,
    active: bool,
    metadata: Metadata,
    user_role: Option<String>,
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

impl Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        let data = extract_scim_user_data(&input.user);
        let metadata = build_scim_metadata(&data, input.active).map_err(failure)?;
        let settings = Settings {
            scim_upsert_user: true,
            scim_admin_group: input.admin_group.clone(),
            default_user_role: input.default_role,
        };
        let default_role = default_scim_user_role(&settings);
        let groups = input.user.groups.as_deref().unwrap_or_default();
        let role = resolve_scim_user_role(groups, input.admin_group.as_deref(), default_role);
        serde_json::to_value(Output {
            user_email: data.user_email,
            user_alias: data.user_alias,
            sso_user_id: data.sso_user_id,
            teams: data.teams,
            active: data.active,
            metadata,
            user_role: role.map(ToString::to_string),
        })
        .map_err(failure)
    }
}
