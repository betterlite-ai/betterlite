use super::claims::{ClaimError, JwtClaims, WhenUnset, field_value, marker_claim, text_value};

const USER_ID_CLAIM: &str = "_litellm_user_id";
const USER_EMAIL_CLAIM: &str = "_litellm_user_email";
const END_USER_ID_CLAIM: &str = "_litellm_end_user_id";

pub trait IdentityClaims {
    fn user_id(&self, default: Option<&str>) -> Result<Option<String>, ClaimError>;
    fn user_email(&self, default: Option<&str>) -> Result<Option<String>, ClaimError>;
    fn end_user_id(&self, default: Option<&str>) -> Result<Option<String>, ClaimError>;
    fn object_id(&self, default: Option<&str>) -> Result<Option<String>, ClaimError>;
    fn agent_claim(&self) -> Result<Option<String>, ClaimError>;
}

impl IdentityClaims for JwtClaims<'_> {
    fn user_id(&self, default: Option<&str>) -> Result<Option<String>, ClaimError> {
        if let Some(value) = marker_claim(self, USER_ID_CLAIM)? {
            return text_value(value, USER_ID_CLAIM);
        }
        let field = self.settings.user_id_jwt_field.as_deref();
        field_value(self, field, default, WhenUnset::UseDefault)
    }

    fn user_email(&self, default: Option<&str>) -> Result<Option<String>, ClaimError> {
        if let Some(value) = marker_claim(self, USER_EMAIL_CLAIM)? {
            return text_value(value, USER_EMAIL_CLAIM);
        }
        let field = self.settings.user_email_jwt_field.as_deref();
        field_value(self, field, default, WhenUnset::Absent)
    }

    fn end_user_id(&self, default: Option<&str>) -> Result<Option<String>, ClaimError> {
        if let Some(value) = marker_claim(self, END_USER_ID_CLAIM)? {
            return text_value(value, END_USER_ID_CLAIM);
        }
        let field = self.settings.end_user_id_jwt_field.as_deref();
        field_value(self, field, default, WhenUnset::Absent)
    }

    fn object_id(&self, default: Option<&str>) -> Result<Option<String>, ClaimError> {
        let field = self.settings.object_id_jwt_field.as_deref();
        field_value(self, field, default, WhenUnset::UseDefault)
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
}
