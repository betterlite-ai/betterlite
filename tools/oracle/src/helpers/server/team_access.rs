use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_auth::management_routes::KEY_MANAGEMENT_ROUTES;
use betterlite_auth::management_routes::key_routes::KeyManagementRoute;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_auth::user_roles::UserRole;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::team_rows::member::MemberRole;
use betterlite_db::team_rows::team::TeamBase;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::team_access::access::{OrgRoles, TeamAccess, TeamRole, is_team_admin};
use betterlite_server::team_access::caller::TeamCaller;
use betterlite_server::team_access::error::TeamAccessError;
use betterlite_server::team_access::permissions::{
    BASELINE_PERMISSIONS, enforce_group_assignment, member_permissions, member_route_allowed,
};
use serde::{Deserialize, Serialize};
use tokio::runtime::Builder;
use tokio::sync::Mutex;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Step {
    function: Function,
    caller: CallerInput,
    team: TeamBase<String>,
    personal_key: bool,
    allow: Vec<TeamRole>,
    org_admins: Vec<(String, String)>,
    role: Option<MemberRole>,
    route: String,
    access_group_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    Allows,
    StrongestRole,
    IsTeamAdmin,
    Permissions,
    MemberRoute,
    AssignGroups,
    DefaultPermissions,
    AllPermissions,
    Denied,
}

// the UserAPIKeyAuth fields the case sets, built into a real key so the production conversion runs
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CallerInput {
    user_id: Option<String>,
    user_role: Option<UserRole>,
    team_id: Option<String>,
    metadata: CallerMetadata,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CallerMetadata {
    service_account_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct StepResult {
    value: Answer,
    error: Option<ErrorOut>,
    org_lookups: Vec<(String, String)>,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum Answer {
    Flag(bool),
    Role(Option<TeamRole>),
    Routes(Vec<&'static str>),
    Nothing,
}

#[derive(Debug, Serialize)]
struct ErrorOut {
    status: Option<u16>,
    error_type: Option<String>,
    param: Option<String>,
    code: Option<String>,
}

// OrgRoles answered from the case's (user_id, organization_id) pairs, recording each lookup
struct CaseOrgRoles {
    admins: Vec<(String, String)>,
    lookups: Mutex<Vec<(String, String)>>,
}

#[async_trait]
impl OrgRoles for CaseOrgRoles {
    async fn is_org_admin(
        &self,
        user_id: &str,
        organization_id: &str,
    ) -> Result<bool, TeamAccessError> {
        let pair = (user_id.to_owned(), organization_id.to_owned());
        self.lookups.lock().await.push(pair);
        Ok(self
            .admins
            .iter()
            .any(|(user, organization)| user == user_id && organization == organization_id))
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread().build()?;
        let mut results = Vec::new();
        for step in &input.steps {
            results.push(runtime.block_on(run_step(step))?);
        }
        Ok(serde_json::to_value(results)?)
    }
}

async fn run_step(step: &Step) -> Result<StepResult, Box<HelperError>> {
    let roles = Arc::new(CaseOrgRoles {
        admins: step.org_admins.clone(),
        lookups: Mutex::new(Vec::new()),
    });
    let org_roles: Arc<dyn OrgRoles> = Arc::<CaseOrgRoles>::clone(&roles);
    let access = TeamAccess { org_roles };
    let fields: BTreeMap<String, JsonData> =
        serde_json::from_value(serde_json::to_value(&step.caller)?)?;
    let auth = UserAPIKeyAuth::construct(AuthInput::Dict(fields)).or_value_error()?;
    let caller = TeamCaller::try_from(&auth).or_value_error()?;
    let outcome = answer(step, &caller, &access).await;
    let org_lookups = roles.lookups.lock().await.clone();
    Ok(match outcome {
        Ok(value) => StepResult {
            value,
            error: None,
            org_lookups,
        },
        Err(error) => StepResult {
            value: Answer::Nothing,
            error: Some(error_out(&error)),
            org_lookups,
        },
    })
}

// the status, type, param and code LiteLLM's raised exception carries; the message is betterlite's own
fn error_out(error: &TeamAccessError) -> ErrorOut {
    let exception = error.proxy_exception();
    ErrorOut {
        status: error.status(),
        error_type: exception.as_ref().map(|found| found.error_type.clone()),
        param: exception.as_ref().and_then(|found| found.param.clone()),
        code: exception.map(|found| found.code),
    }
}

async fn answer(
    step: &Step,
    caller: &TeamCaller,
    access: &TeamAccess,
) -> Result<Answer, TeamAccessError> {
    let team = &step.team;
    match step.function {
        Function::Allows => Ok(Answer::Flag(
            access.allows(caller, team, &step.allow).await?,
        )),
        Function::StrongestRole => Ok(Answer::Role(access.strongest_role(caller, team).await?)),
        Function::IsTeamAdmin => Ok(Answer::Flag(is_team_admin(caller, team))),
        Function::Permissions => {
            let mut routes: Vec<&'static str> = member_permissions(team)?
                .into_iter()
                .map(KeyManagementRoute::value)
                .collect();
            routes.sort_unstable();
            Ok(Answer::Routes(routes))
        }
        Function::MemberRoute => {
            let allowed = member_route_allowed(step.role.as_ref(), team, &step.route)?;
            Ok(Answer::Flag(allowed))
        }
        Function::AssignGroups => {
            let target = (!step.personal_key).then_some(team);
            enforce_group_assignment(caller, target, step.access_group_ids.as_deref())?;
            Ok(Answer::Nothing)
        }
        Function::DefaultPermissions => Ok(Answer::Routes(
            BASELINE_PERMISSIONS
                .into_iter()
                .map(KeyManagementRoute::value)
                .collect(),
        )),
        Function::AllPermissions => Ok(Answer::Routes(KEY_MANAGEMENT_ROUTES.to_vec())),
        Function::Denied => Err(TeamAccessError::Denied),
    }
}
