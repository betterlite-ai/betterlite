use betterlite_auth::auth_identity::errors::AuthError;
use betterlite_auth::auth_identity::records::{
    CredentialRef, EndUserIdentity, NetworkContext, OrganizationIdentity, Principal,
    ProjectIdentity, SourceKey, TeamIdentity, UserIdentity,
};
use betterlite_auth::auth_identity::roles::{
    AuthMethod, PrincipalType, TeamRole, effective_user_role, get_user_role, is_user_proxy_admin,
    map_role, team_role,
};
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use serde::Deserialize;

use crate::compare::helper::HelperError;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "aspect", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Role(RoleInput),
    Records(RecordsInput),
    Principal(PrincipalInput),
    Error(ErrorInput),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RoleInput {
    user_role: Option<String>,
    user_present: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecordsInput {
    id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PrincipalInput {
    subject: String,
}

#[derive(Deserialize)]
#[serde(tag = "variant", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ErrorInput {
    NoDatabaseConnection,
    KeyNotInCache { hashed_token: String },
    KeyNotFound { hashed_token: String },
    PrincipalMissingSourceKey,
    UserNotFound { user_id: String },
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::Role(fields) => run_role(fields),
            Input::Records(fields) => Ok(run_records(fields)),
            Input::Principal(fields) => run_principal(fields),
            Input::Error(fields) => Ok(run_error(fields)),
        }
    }
}

fn serialization_error(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "SerializationError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn user_row(fields: &RoleInput) -> Result<Option<LiteLLMUserTable>, Box<HelperError>> {
    fields
        .user_present
        .then(|| {
            serde_json::from_value(serde_json::json!({
                "user_id": "u1",
                "user_role": fields.user_role,
            }))
        })
        .transpose()
        .map_err(|error| serialization_error(error.to_string()))
}

fn role_fields(value: Option<&str>, user: Option<&LiteLLMUserTable>) -> serde_json::Value {
    serde_json::json!({
        "map_role": map_role(value),
        "team_role": team_role(value),
        "effective_user_role": effective_user_role(value),
        "get_user_role": get_user_role(user),
        "is_user_proxy_admin": is_user_proxy_admin(user),
    })
}

fn run_role(fields: &RoleInput) -> crate::helpers::HelperResult {
    let user = user_row(fields)?;
    let value = fields.user_role.as_deref();
    Ok(role_fields(value, user.as_ref()))
}

fn run_records(fields: &RecordsInput) -> serde_json::Value {
    let user = UserIdentity {
        id: fields.id.clone(),
        external_id: None,
        user_name: None,
        email: None,
        display_name: None,
    };
    let organization = OrganizationIdentity {
        id: fields.id.clone(),
        name: None,
    };
    let project = ProjectIdentity {
        id: fields.id.clone(),
        name: None,
    };
    let team = TeamIdentity {
        id: fields.id.clone(),
        name: None,
        role: TeamRole::default(),
    };
    let end_user = EndUserIdentity {
        id: fields.id.clone(),
    };
    serde_json::json!({
        "user": user,
        "organization": organization,
        "project": project,
        "team": team,
        "end_user": end_user,
        "credential_ref": CredentialRef::default(),
        "network": NetworkContext::default(),
    })
}

fn run_principal(fields: &PrincipalInput) -> crate::helpers::HelperResult {
    let principal = Principal {
        principal_type: PrincipalType::Human,
        subject: fields.subject.clone(),
        issuer: None,
        audience: Vec::new(),
        user: None,
        organization: None,
        teams: Vec::new(),
        project: None,
        end_user: None,
        roles: Vec::new(),
        scopes: Vec::new(),
        auth_method: AuthMethod::ApiKey,
        credential_ref: CredentialRef::default(),
        network: NetworkContext::default(),
        source_key: SourceKey::default(),
    };
    serde_json::to_value(&principal).map_err(|error| serialization_error(error.to_string()))
}

fn run_error(fields: &ErrorInput) -> serde_json::Value {
    let error = match fields {
        ErrorInput::NoDatabaseConnection => AuthError::NoDatabaseConnection,
        ErrorInput::KeyNotInCache { hashed_token } => AuthError::KeyNotInCache {
            hashed_token: hashed_token.clone(),
        },
        ErrorInput::KeyNotFound { hashed_token } => AuthError::KeyNotFound {
            hashed_token: hashed_token.clone(),
        },
        ErrorInput::PrincipalMissingSourceKey => AuthError::PrincipalMissingSourceKey,
        ErrorInput::UserNotFound { user_id } => AuthError::UserNotFound {
            user_id: user_id.clone(),
        },
    };
    let (status, error_type, param, code) =
        error
            .http_mapping()
            .map_or((None, None, None, None), |mapping| {
                (
                    Some(mapping.status),
                    Some(mapping.error_type),
                    Some(mapping.param),
                    Some(mapping.code),
                )
            });
    serde_json::json!({
        "kind": error.kind(),
        "status": status,
        "error_type": error_type,
        "param": param,
        "code": code,
    })
}
