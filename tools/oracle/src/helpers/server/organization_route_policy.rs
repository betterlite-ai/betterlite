mod tests;

use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_auth::organization_route_policy::error::RoutePolicyError;
use betterlite_auth::organization_route_policy::membership::RouteUser;
use betterlite_auth::organization_route_policy::org_admin::user_is_org_admin;
use betterlite_auth::organization_route_policy::role_check::check_organization_role;
use betterlite_auth::organization_route_policy::team_context::{
    TeamOrganizationLookup, add_team_org_context,
};
use betterlite_models::model_registration::JsonObject;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    function: Function,
    user: Option<RouteUser>,
    route: Option<String>,
    #[serde(default)]
    request: JsonObject,
    route_template: Option<String>,
    team_organization_id: Option<String>,
    #[serde(default)]
    lookup_error: bool,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    RoleCheck,
    OrgInfo,
    OrgAdmin,
    TeamContext,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread().build().or_value_error()?;
        let mut answers = Vec::new();
        for call in &input.calls {
            answers.push(match runtime.block_on(answer(call)) {
                Ok(value) => json!({"ok": true, "value": value}),
                Err(_failure) => json!({"ok": false}),
            });
        }
        Ok(Value::Array(answers))
    }
}

async fn answer(call: &Call) -> Result<Value, Failure> {
    match call.function {
        Function::RoleCheck => role_check_value(call),
        Function::OrgInfo => org_info_value(call),
        Function::OrgAdmin => Ok(org_admin_value(call)),
        Function::TeamContext => team_context_value(call).await,
    }
}

fn role_check_value(call: &Call) -> Result<Value, Failure> {
    let route = call.route.as_deref().ok_or(Failure::MissingRoute)?;
    let denial = match check_organization_role(call.user.as_ref(), route, &call.request) {
        Ok(()) => Value::Null,
        Err(error) => denial_json(&error),
    };
    Ok(json!({"denied": denial}))
}

// get_user_organization_info's pair, with the role map the pinned code builds from the memberships
fn org_info_value(call: &Call) -> Result<Value, Failure> {
    let user = call.user.as_ref().ok_or(Failure::MissingUser)?;
    let info = user.organization_info();
    Ok(json!({"organizations": info.organizations, "roles": info.roles}))
}

fn org_admin_value(call: &Call) -> Value {
    json!({"admin": user_is_org_admin(&call.request, call.user.as_ref())})
}

// the effective body the route gate reads, plus the team ids the pinned function asked to look up
async fn team_context_value(call: &Call) -> Result<Value, Failure> {
    let route = call.route.as_deref().ok_or(Failure::MissingRoute)?;
    let lookup = ScriptedLookup {
        organization_id: call.team_organization_id.clone(),
        fail: call.lookup_error,
        fetches: Mutex::new(Vec::new()),
    };
    let body = add_team_org_context(
        route,
        &call.request,
        &lookup,
        call.route_template.as_deref(),
    )
    .await
    .map_err(Failure::Lookup)?;
    let fetches = lookup
        .fetches
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner);
    Ok(json!({"body": body, "fetches": fetches}))
}

// the ProxyException fields the auth error handler turns into the response body
fn denial_json(error: &RoutePolicyError) -> Value {
    let exception = error.clone().into_proxy_exception();
    json!({
        "type": exception.error_type,
        "param": exception.param,
        "code": exception.code,
    })
}

struct ScriptedLookup {
    organization_id: Option<String>,
    fail: bool,
    fetches: Mutex<Vec<String>>,
}

#[async_trait]
impl TeamOrganizationLookup for ScriptedLookup {
    type Failure = LookupFailure;

    async fn team_organization_id(&self, team_id: &str) -> Result<Option<String>, Self::Failure> {
        self.fetches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(team_id.to_owned());
        if self.fail {
            return Err(LookupFailure {
                team_id: team_id.to_owned(),
            });
        }
        Ok(self.organization_id.clone())
    }
}

#[derive(Debug, thiserror::Error)]
enum Failure {
    #[error("the case names no route")]
    MissingRoute,
    #[error("this answer needs a user")]
    MissingUser,
    #[error(transparent)]
    Lookup(LookupFailure),
}

#[derive(Debug, thiserror::Error)]
#[error("the scripted team lookup failed for {team_id}")]
struct LookupFailure {
    team_id: String,
}
