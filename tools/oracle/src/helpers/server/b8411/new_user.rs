use betterlite_auth::user_roles::UserRole;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_server::key_fields::values::Amount;
use betterlite_server::proxy_request_contracts::user::{NewUserRequest, NewUserTeams};
use serde::Serialize;
use serde_json::Value;

#[derive(Serialize)]
struct NewUserView<'a> {
    user_email: Option<&'a String>,
    user_alias: Option<&'a String>,
    user_role: Option<UserRole>,
    max_budget: Option<&'a Amount>,
    teams: Option<&'a NewUserTeams>,
    auto_create_key: bool,
    send_invite_email: Option<bool>,
    sso_user_id: Option<&'a String>,
    organizations: Option<&'a Vec<String>>,
}

pub(super) fn answer(body: &Value) -> Result<Value, Box<HelperError>> {
    let request = NewUserRequest::from_value(body).or_value_error()?;
    let view = NewUserView {
        user_email: request.user_email.as_ref(),
        user_alias: request.user_alias.as_ref(),
        user_role: request.user_role,
        max_budget: request.max_budget.as_ref(),
        teams: request.teams.as_ref(),
        auto_create_key: request.auto_create_key,
        send_invite_email: request.send_invite_email,
        sso_user_id: request.sso_user_id.as_ref(),
        organizations: request.organizations.as_ref(),
    };
    serde_json::to_value(view).or_value_error()
}
