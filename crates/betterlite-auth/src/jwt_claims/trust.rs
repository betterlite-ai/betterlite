use super::claims::JwtClaims;

const ISSUER_MARKER_CLAIM: &str = "_litellm_jwt_issuer";

pub trait TrustClaims {
    fn is_trusted_marker_token(&self) -> bool;
    fn has_trusted_claim(&self, claim: &str) -> bool;
}

impl TrustClaims for JwtClaims<'_> {
    fn is_trusted_marker_token(&self) -> bool {
        let Some(value) = self.claims.json.get(ISSUER_MARKER_CLAIM) else {
            return false;
        };
        let Some(issuer) = value.as_str() else {
            return false;
        };
        if issuer.is_empty() {
            return false;
        }
        self.trusted_issuers
            .iter()
            .any(|configured| configured == issuer)
    }

    fn has_trusted_claim(&self, claim: &str) -> bool {
        if !self.is_trusted_marker_token() {
            return false;
        }
        self.claims.json.get(claim).is_some()
    }
}
