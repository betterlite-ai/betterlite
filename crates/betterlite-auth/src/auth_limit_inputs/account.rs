use secrecy::ExposeSecret;

use crate::resolved_key::auth::UserAPIKeyAuth;

// The caller identity and organization ceilings the rate-limit descriptor builders key their counters by.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AccountLimits<'auth> {
    pub hashed_api_key: Option<&'auth str>,
    pub org_id: Option<&'auth str>,
    pub team_id: Option<&'auth str>,
    pub project_id: Option<&'auth str>,
    pub organization_rpm_limit: Option<i64>,
    pub organization_tpm_limit: Option<i64>,
}

// The type keeps its one impl block in resolved_key, so the readers are a free function.
#[must_use]
pub fn account_limits(auth: &UserAPIKeyAuth) -> AccountLimits<'_> {
    AccountLimits {
        hashed_api_key: auth
            .extra
            .api_key
            .0
            .as_ref()
            .map(ExposeSecret::expose_secret),
        org_id: auth.view.token.org_id.as_deref(),
        team_id: auth.view.token.team_id.as_deref(),
        project_id: auth.view.token.project_id.as_deref(),
        organization_rpm_limit: auth.view.organization_rpm_limit,
        organization_tpm_limit: auth.view.organization_tpm_limit,
    }
}
