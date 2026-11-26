use betterlite_db::database_availability::policy::AllowRequestsOnOutage;
use betterlite_db::team_rows::team::LiteLLMTeamTable;

use crate::auth_org_lookup::error::OrgLookupError;
use crate::auth_org_lookup::last_known::get_org_object_for_request;
use crate::auth_org_lookup::organization::OrgLookup;
use crate::resolved_key::auth::UserAPIKeyAuth;

// _inherit_org_identity: take the id from the team, then copy the org's identity and budget onto a token that has none
pub async fn inherit_org_identity(
    token: &mut UserAPIKeyAuth,
    team: Option<&LiteLLMTeamTable>,
    lookup: &OrgLookup,
    outage: &AllowRequestsOnOutage,
) -> Result<(), OrgLookupError> {
    inherit_org_id(token, team);
    if org_identity_is_populated(token) || lookup.store.is_none() {
        return Ok(());
    }
    let Some(org_id) = token.view.token.org_id.clone() else {
        return Ok(());
    };
    let Some(org) = get_org_object_for_request(lookup, &org_id, outage).await? else {
        return Ok(());
    };
    token.view.organization_alias = org.organization_alias;
    token.view.organization_metadata = org.metadata;
    let Some(budget) = org.litellm_budget_table else {
        return Ok(());
    };
    token.view.organization_max_budget = budget.max_budget;
    token.view.organization_tpm_limit = budget.tpm_limit;
    token.view.organization_rpm_limit = budget.rpm_limit;
    Ok(())
}

fn inherit_org_id(token: &mut UserAPIKeyAuth, team: Option<&LiteLLMTeamTable>) {
    if token.view.token.org_id.is_some() {
        return;
    }
    let Some(organization_id) = team.and_then(|row| row.fields.base.organization_id.as_ref())
    else {
        return;
    };
    token.view.token.org_id = Some(organization_id.clone());
}

fn org_identity_is_populated(token: &UserAPIKeyAuth) -> bool {
    token.view.organization_alias.is_some()
        || token.view.organization_max_budget.is_some()
        || token.view.organization_tpm_limit.is_some()
        || token.view.organization_rpm_limit.is_some()
        || token.view.organization_metadata.is_some()
}
