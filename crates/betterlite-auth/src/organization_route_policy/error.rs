use std::collections::BTreeMap;

use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use thiserror::Error;

const AUTH_ERROR: &str = "auth_error";
const INVALID_REQUEST_ERROR: &str = "invalid_request_error";
const ORGANIZATION_ID: &str = "organization_id";
const USER_ROLE: &str = "user_role";
const UNAUTHORIZED: i64 = 401;
const BAD_REQUEST: i64 = 400;

// the 401 auth_error rejections and their param; OrganizationIdNotText is the 400 for a wrong-typed value
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum RoutePolicyError {
    #[error("Only proxy admins can create new organizations. You are {role}")]
    OrganizationCreateDenied { role: String },
    #[error(
        "Tried to access route={route} but you are not a member of any organization. Please contact the proxy admin to request access."
    )]
    NotAMember { route: String },
    #[error("Passed organization_id is None, please pass an organization_id in your request")]
    MissingOrganizationId,
    #[error(
        "You do not have a role within the selected organization. Passed organization_id: {organization_id}. Please contact the organization admin to request access."
    )]
    NoRoleInOrganization { organization_id: String },
    #[error(
        "You do not have the required role to perform {route} in Organization {organization_id}. Your role is {role} in Organization {organization_id}"
    )]
    NotOrganizationAdmin {
        route: String,
        organization_id: String,
        role: String,
    },
    #[error(
        "Passed organization_id is None, please specify the organization_id in your request. You are part of multiple organizations: {organizations}"
    )]
    TeamCreateNeedsOrganization { organizations: String },
    #[error(
        "You do not have the required role to call {route}. Your role is {role} in Organization {organization_id}"
    )]
    TeamCreateNeedsOrgAdmin {
        route: String,
        organization_id: String,
        role: String,
    },
    #[error("organization_id must be a string or null")]
    OrganizationIdNotText,
}

impl RoutePolicyError {
    // LiteLLM raises these as ProxyException, and B749's denial_response renders type, param and code unchanged
    #[must_use]
    pub fn into_proxy_exception(self) -> ProxyException {
        let (error_type, param, code) = match &self {
            Self::OrganizationCreateDenied { .. }
            | Self::NotOrganizationAdmin { .. }
            | Self::TeamCreateNeedsOrgAdmin { .. } => (AUTH_ERROR, USER_ROLE, UNAUTHORIZED),
            Self::NotAMember { .. }
            | Self::MissingOrganizationId
            | Self::NoRoleInOrganization { .. }
            | Self::TeamCreateNeedsOrganization { .. } => {
                (AUTH_ERROR, ORGANIZATION_ID, UNAUTHORIZED)
            }
            Self::OrganizationIdNotText => (INVALID_REQUEST_ERROR, ORGANIZATION_ID, BAD_REQUEST),
        };
        ProxyException::new(ProxyExceptionInit {
            message: self.to_string(),
            error_type: error_type.to_owned(),
            param: Some(param.to_owned()),
            code: ExceptionCode::Status(code),
            headers: BTreeMap::new(),
            openai_code: None,
            provider_specific_fields: BTreeMap::new(),
        })
    }
}
