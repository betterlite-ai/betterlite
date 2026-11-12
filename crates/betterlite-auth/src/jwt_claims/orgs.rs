use super::claims::{ClaimError, JwtClaims, WhenUnset, field_value, marker_claim, text_value};

const ORG_ID_CLAIM: &str = "_litellm_org_id";

pub trait OrgClaims {
    fn org_id(&self, default: Option<&str>) -> Result<Option<String>, ClaimError>;
    fn org_alias(&self, default: Option<&str>) -> Result<Option<String>, ClaimError>;
}

impl OrgClaims for JwtClaims<'_> {
    fn org_id(&self, default: Option<&str>) -> Result<Option<String>, ClaimError> {
        if let Some(value) = marker_claim(self, ORG_ID_CLAIM)? {
            return text_value(value, ORG_ID_CLAIM);
        }
        let field = self.settings.org_id_jwt_field.as_deref();
        field_value(self, field, default, WhenUnset::Absent)
    }

    fn org_alias(&self, default: Option<&str>) -> Result<Option<String>, ClaimError> {
        let field = self.settings.org_alias_jwt_field.as_deref();
        field_value(self, field, default, WhenUnset::Absent)
    }
}
