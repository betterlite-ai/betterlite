use std::io::Write;

use rust_decimal::Decimal;
use serde::Serialize;

use super::{Client, CreateUser, UsersError, post_user};

const NEW_PATH: &str = "/user/new";

#[derive(Serialize)]
struct CreateUserBody<'request> {
    user_email: &'request str,
    user_role: &'request str,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_alias: Option<&'request str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    teams: Option<&'request [String]>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        with = "rust_decimal::serde::arbitrary_precision_option"
    )]
    max_budget: Option<Decimal>,
}

pub(super) fn create_user(
    options: &CreateUser,
    client: &Client,
    stdout: &mut dyn Write,
) -> Result<(), UsersError> {
    let teams = (!options.team.is_empty()).then_some(options.team.as_slice());
    let body = serde_json::to_vec(&CreateUserBody {
        user_email: &options.email,
        user_role: &options.role,
        user_alias: options.alias.as_deref(),
        teams,
        max_budget: options.max_budget,
    })
    .map_err(UsersError::Encode)?;
    post_user(client, NEW_PATH, body, stdout)
}
