use super::claims::{ClaimError, JwtClaims, text_value};
use super::trust::TrustClaims as _;

const USER_ID_CLAIM: &str = "_litellm_user_id";
const USER_EMAIL_CLAIM: &str = "_litellm_user_email";
const END_USER_ID_CLAIM: &str = "_litellm_end_user_id";

pub trait IdentityClaims {
    fn user_id(&self, default: Option<&str>) -> Result<Option<String>, ClaimError>;
    fn user_email(&self, default: Option<&str>) -> Result<Option<String>, ClaimError>;
    fn end_user_id(&self, default: Option<&str>) -> Result<Option<String>, ClaimError>;
    fn object_id(&self, default: Option<&str>) -> Result<Option<String>, ClaimError>;
    fn agent_claim(&self) -> Result<Option<String>, ClaimError>;
    fn is_upsert_user_id(&self, valid_user_email: Option<bool>) -> bool;
    fn is_enforced_email_domain(&self) -> bool;
}

impl IdentityClaims for JwtClaims<'_> {
    fn user_id(&self, default: Option<&str>) -> Result<Option<String>, ClaimError> {
        if self.has_trusted_claim(USER_ID_CLAIM) {
            let Some(value) = self.nested(USER_ID_CLAIM)? else {
                return Ok(None);
            };
            return text_value(value, USER_ID_CLAIM);
        }
        let Some(field) = self.settings.user_id_jwt_field.as_deref() else {
            return Ok(default.map(str::to_owned));
        };
        self.nested_text(field, default)
    }

    fn user_email(&self, default: Option<&str>) -> Result<Option<String>, ClaimError> {
        if self.has_trusted_claim(USER_EMAIL_CLAIM) {
            let Some(value) = self.nested(USER_EMAIL_CLAIM)? else {
                return Ok(None);
            };
            return text_value(value, USER_EMAIL_CLAIM);
        }
        let Some(field) = self.settings.user_email_jwt_field.as_deref() else {
            return Ok(None);
        };
        self.nested_text(field, default)
    }

    fn end_user_id(&self, default: Option<&str>) -> Result<Option<String>, ClaimError> {
        if self.has_trusted_claim(END_USER_ID_CLAIM) {
            let Some(value) = self.nested(END_USER_ID_CLAIM)? else {
                return Ok(None);
            };
            return text_value(value, END_USER_ID_CLAIM);
        }
        let Some(field) = self.settings.end_user_id_jwt_field.as_deref() else {
            return Ok(None);
        };
        self.nested_text(field, default)
    }

    fn object_id(&self, default: Option<&str>) -> Result<Option<String>, ClaimError> {
        let Some(field) = self.settings.object_id_jwt_field.as_deref() else {
            return Ok(default.map(str::to_owned));
        };
        self.nested_text(field, default)
    }

    fn agent_claim(&self) -> Result<Option<String>, ClaimError> {
        let Some(field) = self.settings.agent_id_jwt_field.as_deref() else {
            return Ok(None);
        };
        let Some(value) = self.nested(field)? else {
            return Ok(None);
        };
        let Some(claim) = value.json.as_str() else {
            return Ok(None);
        };
        if claim.is_empty() {
            return Ok(None);
        }
        Ok(Some(claim.to_owned()))
    }

    fn is_upsert_user_id(&self, valid_user_email: Option<bool>) -> bool {
        if valid_user_email == Some(false) {
            return false;
        }
        self.settings.user_id_upsert
    }

    fn is_enforced_email_domain(&self) -> bool {
        if self.settings.user_allowed_email_domain.is_none() {
            return false;
        }
        true
    }
}
