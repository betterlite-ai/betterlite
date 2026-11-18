#![cfg(test)]

mod budget;
mod ownership;
mod permissions;
mod routes;

use betterlite_auth::user_roles::UserRole;
use betterlite_db::budget_permission_rows::JsonData;

use crate::key_fields::base::KeyRequestBase;
use crate::key_fields::decode::decode_key_request;

use super::caller::Caller;
use super::request::{KeyDelegationRequest, Sent};

const fn caller(role: Option<UserRole>) -> Caller {
    Caller {
        role,
        is_session_token: false,
        is_ui_session_team_key: false,
    }
}

const fn admin() -> Caller {
    caller(Some(UserRole::ProxyAdmin))
}

const fn user() -> Caller {
    caller(Some(UserRole::InternalUser))
}

const fn cli_session() -> Caller {
    Caller {
        role: Some(UserRole::InternalUser),
        is_session_token: true,
        is_ui_session_team_key: false,
    }
}

const fn ui_session_team_key() -> Caller {
    Caller {
        role: Some(UserRole::InternalUser),
        is_session_token: true,
        is_ui_session_team_key: true,
    }
}

const fn sent<T>(supplied: bool, value: Option<T>) -> Sent<T> {
    Sent { supplied, value }
}

fn json(text: &str) -> JsonData {
    serde_json::from_str(text).expect("fixture JSON")
}

fn presets(routes: &[&str]) -> Vec<JsonData> {
    routes
        .iter()
        .map(|route| JsonData::String((*route).to_owned()))
        .collect()
}

// The production projection keeps pydantic's model_fields_set beside the value, so tests decode real bodies.
fn projected(body: &str) -> KeyDelegationRequest {
    let value = decode_key_request(body.as_bytes()).expect("a key request object");
    let request = KeyRequestBase::read(&value).expect("the key request fields");
    KeyDelegationRequest::from(&request)
}
