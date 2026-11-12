use super::claims::{ClaimError, JwtClaims, text_value};
use super::trust::TrustClaims as _;

const ORG_ID_CLAIM: &str = "_litellm_org_id";

pub trait OrgClaims {
    fn org_id(&self, default: Option<&str>) -> Result<Option<String>, ClaimError>;
    fn org_alias(&self, default: Option<&str>) -> Result<Option<String>, ClaimError>;
}

impl OrgClaims for JwtClaims<'_> {
    fn org_id(&self, default: Option<&str>) -> Result<Option<String>, ClaimError> {
        if self.has_trusted_claim(ORG_ID_CLAIM) {
            let Some(value) = self.nested(ORG_ID_CLAIM)? else {
                return Ok(None);
            };
            return text_value(value, ORG_ID_CLAIM);
        }
        let Some(field) = self.settings.org_id_jwt_field.as_deref() else {
            return Ok(None);
        };
        self.nested_text(field, default)
    }

    fn org_alias(&self, default: Option<&str>) -> Result<Option<String>, ClaimError> {
        let Some(field) = self.settings.org_alias_jwt_field.as_deref() else {
            return Ok(None);
        };
        self.nested_text(field, default)
    }
}
