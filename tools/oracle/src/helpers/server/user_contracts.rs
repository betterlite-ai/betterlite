use std::collections::BTreeSet;

use betterlite_auth::user_roles::UserRole;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::key_fields::values::Amount;
use betterlite_server::proxy_request_contracts::user::NewUserTeams;
use betterlite_server::user_contracts::bulk_delete::{
    BulkDeleteUserRequest, BulkDeleteUsersResponse, UserDeleteResult,
};
use betterlite_server::user_contracts::bulk_new::{
    BulkNewUserMeta, BulkNewUserRequest, BulkNewUserResponse, UserCreateResult,
};
use betterlite_server::user_contracts::bulk_update::{
    BulkUpdateUserRequest, BulkUpdateUserResponse, UserUpdateResult,
};
use betterlite_server::user_contracts::decode_user_info_query;
use betterlite_server::user_contracts::list::UserListResponse;
use betterlite_server::user_contracts::query::user_id_from_query;
use betterlite_server::user_contracts::search::UserSearchWhere;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    BulkUpdate,
    BulkDelete,
    BulkNew,
    UserUpdateResult,
    BulkUpdateResponse,
    UserDeleteResult,
    BulkDeleteResponse,
    UserCreateResult,
    BulkNewMeta,
    BulkNewResponse,
    UserListResponse,
    UserSearchWhere,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    function: Function,
    body: JsonData,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QueryStep {
    query: String,
    supplied: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    calls: Vec<Call>,
    #[serde(default)]
    queries: Vec<QueryStep>,
}

#[derive(Serialize)]
struct Outcome {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<Value>,
}

#[derive(Serialize)]
struct QueryOutcome {
    resolved: Option<String>,
    normalized: Option<String>,
}

#[derive(Serialize)]
struct Answers {
    calls: Vec<Outcome>,
    queries: Vec<QueryOutcome>,
}

#[derive(Serialize)]
struct UpdateUserView<'record> {
    user_id: Option<&'record String>,
    user_email: Option<&'record String>,
    supplied: &'record BTreeSet<String>,
}

#[derive(Serialize)]
struct BulkUpdateView<'record> {
    users: Option<Vec<UpdateUserView<'record>>>,
    all_users: Option<bool>,
    user_updates: Option<&'record BTreeSet<String>>,
}

#[derive(Serialize)]
struct NewUserItemView<'record> {
    user_id: Option<String>,
    user_email: Option<&'record String>,
    user_alias: Option<&'record String>,
    user_role: Option<UserRole>,
    max_budget: Option<&'record Amount>,
    teams: Option<&'record NewUserTeams>,
    auto_create_key: bool,
    send_invite_email: Option<bool>,
    sso_user_id: Option<&'record String>,
    organizations: Option<&'record Vec<String>>,
}

#[derive(Serialize)]
struct BulkNewView<'record> {
    users: Vec<NewUserItemView<'record>>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let answers = Answers {
            calls: input.calls.iter().map(run_call).collect(),
            queries: input.queries.iter().map(run_query).collect(),
        };
        serde_json::to_value(answers).or_value_error()
    }
}

fn run_call(call: &Call) -> Outcome {
    let value = evaluate(call).ok();
    Outcome {
        ok: value.is_some(),
        value,
    }
}

fn run_query(step: &QueryStep) -> QueryOutcome {
    QueryOutcome {
        resolved: user_id_from_query(&step.query),
        normalized: decode_user_info_query(&step.query, step.supplied.as_deref()),
    }
}

fn evaluate(call: &Call) -> Result<Value, Box<HelperError>> {
    let body = serde_json::to_value(&call.body).or_value_error()?;
    match call.function {
        Function::BulkUpdate => bulk_update_answer(&body),
        Function::BulkDelete => {
            let request = BulkDeleteUserRequest::from_value(&body).or_value_error()?;
            serde_json::to_value(request).or_value_error()
        }
        Function::BulkNew => bulk_new_answer(&body),
        Function::UserUpdateResult => record_answer::<UserUpdateResult>(&body),
        Function::BulkUpdateResponse => record_answer::<BulkUpdateUserResponse>(&body),
        Function::UserDeleteResult => record_answer::<UserDeleteResult>(&body),
        Function::BulkDeleteResponse => record_answer::<BulkDeleteUsersResponse>(&body),
        Function::UserCreateResult => record_answer::<UserCreateResult>(&body),
        Function::BulkNewMeta => record_answer::<BulkNewUserMeta>(&body),
        Function::BulkNewResponse => record_answer::<BulkNewUserResponse>(&body),
        Function::UserListResponse => record_answer::<UserListResponse>(&body),
        Function::UserSearchWhere => {
            let clause = UserSearchWhere::from_value(&body).or_value_error()?;
            serde_json::to_value(clause).or_value_error()
        }
    }
}

fn record_answer<T>(body: &Value) -> Result<Value, Box<HelperError>>
where
    T: DeserializeOwned + Serialize,
{
    let record: T = serde_json::from_value(body.clone()).or_value_error()?;
    serde_json::to_value(record).or_value_error()
}

fn bulk_update_answer(body: &Value) -> Result<Value, Box<HelperError>> {
    let request = BulkUpdateUserRequest::from_value(body).or_value_error()?;
    let users = request.users.as_ref().map(|users| {
        users
            .iter()
            .map(|user| UpdateUserView {
                user_id: user.user_id.as_ref(),
                user_email: user.user_email.as_ref(),
                supplied: &user.fields.supplied,
            })
            .collect()
    });
    let user_updates = request
        .user_updates
        .as_ref()
        .map(|updates| &updates.supplied);
    let view = BulkUpdateView {
        users,
        all_users: request.all_users,
        user_updates,
    };
    serde_json::to_value(view).or_value_error()
}

fn bulk_new_answer(body: &Value) -> Result<Value, Box<HelperError>> {
    let request = BulkNewUserRequest::from_value(body).or_value_error()?;
    let users = request
        .users
        .iter()
        .map(|item| {
            let row = &item.request;
            NewUserItemView {
                user_id: row.base.user_id.clone().flatten(),
                user_email: row.user_email.as_ref(),
                user_alias: row.user_alias.as_ref(),
                user_role: row.user_role,
                max_budget: row.max_budget.as_ref(),
                teams: row.teams.as_ref(),
                auto_create_key: row.auto_create_key,
                send_invite_email: row.send_invite_email,
                sso_user_id: row.sso_user_id.as_ref(),
                organizations: row.organizations.as_ref(),
            }
        })
        .collect();
    serde_json::to_value(BulkNewView { users }).or_value_error()
}
