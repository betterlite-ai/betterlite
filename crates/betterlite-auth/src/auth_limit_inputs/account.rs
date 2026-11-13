use secrecy::ExposeSecret;

use crate::resolved_key::auth::UserAPIKeyAuth;

// The caller identity and organization ceilings the rate-limit descriptor builders key their counters by.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AccountLimits<'auth> {
    pub api_key: Option<&'auth str>,
    pub org_id: Option<&'auth str>,
    pub team_id: Option<&'auth str>,
    pub project_id: Option<&'auth str>,
    pub organization_rpm_limit: Option<i64>,
    pub organization_tpm_limit: Option<i64>,
}

impl UserAPIKeyAuth {
    #[must_use]
    pub fn account_limits(&self) -> AccountLimits<'_> {
        AccountLimits {
            api_key: self
                .extra
                .api_key
                .0
                .as_ref()
                .map(ExposeSecret::expose_secret),
            org_id: self.view.token.org_id.as_deref(),
            team_id: self.view.token.team_id.as_deref(),
            project_id: self.view.token.project_id.as_deref(),
            organization_rpm_limit: self.view.organization_rpm_limit,
            organization_tpm_limit: self.view.organization_tpm_limit,
        }
    }
}
