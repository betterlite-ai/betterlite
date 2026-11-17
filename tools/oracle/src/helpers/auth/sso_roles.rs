use betterlite_auth::jwt_claims::TypedValue;
use betterlite_auth::sso_types::records::{UiAccessMode, check_is_admin_only_access};
use betterlite_auth::sso_types::roles::{
    has_admin_ui_access, is_valid_litellm_user_role, role_from_claim,
};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    claims: Vec<TypedValue>,
    role_names: Vec<String>,
    ui_roles: Vec<String>,
    access_modes: Vec<UiAccessMode>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let roles = input
            .claims
            .iter()
            .map(|claim| role_from_claim(claim).map(|role| role.to_string()))
            .collect::<Vec<Option<String>>>();
        let valid = input
            .role_names
            .iter()
            .map(|name| is_valid_litellm_user_role(name.as_str()))
            .collect::<Vec<bool>>();
        let admin_ui = input
            .ui_roles
            .iter()
            .map(|role| has_admin_ui_access(role.as_str()))
            .collect::<Vec<bool>>();
        let admin_only = input
            .access_modes
            .iter()
            .map(check_is_admin_only_access)
            .collect::<Vec<bool>>();
        Ok(json!({"roles": roles, "valid": valid, "admin_ui": admin_ui, "admin_only": admin_only}))
    }
}
