use betterlite_error::proxy_error_types::ProxyErrorTypes;
use betterlite_models::controls::json_value::JsonValue;
use thiserror::Error;

use crate::oauth_client::document::OAuthUserInfo;
use crate::oauth_client::error::OAuthErrorResponse;
use crate::sso_types::identity::OpenIdIdentity;

pub const GOOGLE_DISCOVERY_URL: &str =
    "https://accounts.google.com/.well-known/openid-configuration";
pub const GOOGLE_PROVIDER: &str = "google";
pub const GOOGLE_SCOPES: [&str; 3] = ["openid", "email", "profile"];

#[derive(Debug, Error)]
pub enum GoogleIdentityError {
    #[error("User {email} is not verified with Google")]
    Unverified { email: String },
    #[error("the Google userinfo field '{field}' is not a {expected}")]
    FieldType {
        field: &'static str,
        expected: &'static str,
    },
}

impl GoogleIdentityError {
    #[must_use]
    pub const fn response(&self) -> OAuthErrorResponse {
        match self {
            Self::Unverified { .. } => OAuthErrorResponse {
                status: 401,
                error_type: ProxyErrorTypes::AuthError,
                param: None,
                code: "401",
            },
            Self::FieldType { field, .. } => OAuthErrorResponse {
                status: 503,
                error_type: ProxyErrorTypes::AuthProviderUnavailable,
                param: Some(*field),
                code: "503",
            },
        }
    }
}

pub fn google_open_id(userinfo: &OAuthUserInfo) -> Result<OpenIdIdentity, GoogleIdentityError> {
    let verified = match userinfo.fields.get("email_verified") {
        None | Some(JsonValue::Null) => false,
        Some(JsonValue::Bool(flag)) => *flag,
        Some(_) => {
            return Err(GoogleIdentityError::FieldType {
                field: "email_verified",
                expected: "boolean",
            });
        }
    };
    if !verified {
        let email = match userinfo.fields.get("email") {
            Some(JsonValue::Text(text)) => text.clone(),
            _ => "None".to_owned(),
        };
        return Err(GoogleIdentityError::Unverified { email });
    }
    Ok(OpenIdIdentity {
        id: string_field(userinfo, "sub")?,
        email: string_field(userinfo, "email")?,
        first_name: string_field(userinfo, "given_name")?,
        last_name: string_field(userinfo, "family_name")?,
        display_name: string_field(userinfo, "name")?,
        picture: string_field(userinfo, "picture")?,
        provider: Some(GOOGLE_PROVIDER.to_owned()),
        team_ids: Vec::new(),
        user_role: None,
        extra_fields: None,
    })
}

fn string_field(
    userinfo: &OAuthUserInfo,
    field: &'static str,
) -> Result<Option<String>, GoogleIdentityError> {
    match userinfo.fields.get(field) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Text(text)) => Ok(Some(text.clone())),
        Some(_) => Err(GoogleIdentityError::FieldType {
            field,
            expected: "string",
        }),
    }
}
