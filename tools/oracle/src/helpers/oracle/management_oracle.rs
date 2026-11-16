use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use std::hint;

mod auth;
mod database;
mod jwt;
mod keys;
mod models;
mod teams;
mod tests;
mod users;

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(tag = "domain", content = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    Models(models::Operation),
    Auth(auth::Operation),
    Teams(teams::Operation),
    Users(users::Operation),
    Keys(keys::Operation),
    Jwt(jwt::Operation),
    Database(database::Operation),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        run(input)
    }
}

pub(crate) fn run(input: &Input) -> HelperResult {
    match input {
        Input::Models(operation) => models::run(operation),
        Input::Auth(operation) => {
            hint::black_box(auth::Operation::Reserved);
            match operation {
                auth::Operation::Reserved => unbuilt("auth"),
            }
        }
        Input::Teams(operation) => {
            hint::black_box(teams::Operation::Reserved);
            match operation {
                teams::Operation::Reserved => unbuilt("teams"),
            }
        }
        Input::Users(operation) => {
            hint::black_box(users::Operation::Reserved);
            match operation {
                users::Operation::Reserved => unbuilt("users"),
            }
        }
        Input::Keys(operation) => {
            hint::black_box(keys::Operation::Reserved);
            match operation {
                keys::Operation::Reserved => unbuilt("keys"),
            }
        }
        Input::Jwt(operation) => {
            hint::black_box(jwt::Operation::Reserved);
            match operation {
                jwt::Operation::Reserved => unbuilt("jwt"),
            }
        }
        Input::Database(operation) => {
            hint::black_box(database::Operation::Reserved);
            match operation {
                database::Operation::Reserved => unbuilt("database"),
            }
        }
    }
}

fn unbuilt(domain: &str) -> HelperResult {
    let message = format!("the {domain} management domain is reserved for its task owner");
    let error = HelperError {
        class: "NotBuilt".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    };
    Err(Box::new(error))
}
