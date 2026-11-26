use std::sync::Arc;

use serde_json::{Value, json};

use betterlite_auth::auth_org_lookup::organization::OrgLookup;
use betterlite_auth::auth_org_lookup::ports::{OrgCache, OrgStore};
use betterlite_auth::auth_policy_support::fetch::safe_fetch;
use betterlite_auth::auth_policy_support::org::inherit_org_identity;
use betterlite_auth::auth_policy_support::team::{
    TeamFromTokenError, team_obj_from_token, token_can_vouch_for_team,
};
use betterlite_auth::auth_team_lookup::error::TeamLookupError;
use betterlite_db::database_availability::policy::{AllowRequestsOnOutage, PolicyError};
use betterlite_db::resource_rows::organization::OrganizationTable;
use betterlite_error::proxy_exception::ProxyException;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::HelperResult;

use super::fixtures::{FakeOrg, org_fields, team_fields};
use super::{Failure, Input, Lookup, invalid, proxy_denial, required_token, runtime, team_obj};

pub(super) fn team_from_token(input: &Input) -> HelperResult {
    let token = required_token(input.token.as_ref())?;
    match team_obj_from_token(&token) {
        Ok(cached) => Ok(team_fields(&cached.team.fields)),
        Err(error) => Ok(team_error(&error)),
    }
}

fn team_error(error: &TeamFromTokenError) -> Value {
    match error {
        TeamFromTokenError::MissingTeamId => json!({ "error": "missing_team_id" }),
        TeamFromTokenError::NonStringModel { index } => {
            json!({ "error": "non_string_model", "index": index })
        }
    }
}

pub(super) fn vouch(input: &Input) -> HelperResult {
    let lookup = input
        .lookup
        .ok_or_else(|| invalid("the call needs a lookup"))?;
    let token = required_token(input.token.as_ref())?;
    let outage = AllowRequestsOnOutage::Boolean(input.outage);
    let value = token_can_vouch_for_team(&token, &lookup_error(lookup), &outage);
    Ok(vouch_json(value))
}

fn lookup_error(lookup: Lookup) -> TeamLookupError {
    match lookup {
        Lookup::TeamNotFound => TeamLookupError::TeamNotFound {
            team_id: "team-x".to_owned(),
        },
        Lookup::Degraded => TeamLookupError::NoDatabase,
    }
}

fn vouch_json(outcome: Result<bool, PolicyError>) -> Value {
    match outcome {
        Ok(value) => json!({ "value": value }),
        Err(PolicyError::UnsupportedValue) => json!({ "error": "unsupported_value" }),
    }
}

pub(super) fn org_inherit(input: &Input) -> HelperResult {
    let mut token = required_token(input.token.as_ref())?;
    let team = team_obj(input.team.as_ref())?;
    let org: Option<OrganizationTable> = input
        .org
        .as_ref()
        .map(|value| serde_json::from_value(value.clone()).or_value_error())
        .transpose()?;
    let cache = Arc::new(FakeOrg::new(org));
    let cache_trait: Arc<dyn OrgCache> = Arc::<FakeOrg>::clone(&cache);
    let wired: Arc<dyn OrgStore> = Arc::<FakeOrg>::clone(&cache);
    let store = input.database.then_some(wired);
    let lookup = OrgLookup {
        store,
        cache: cache_trait,
        in_memory_ttl: 60.0_f64,
    };
    let outage = AllowRequestsOnOutage::Boolean(false);
    runtime()?
        .block_on(inherit_org_identity(
            &mut token,
            team.as_ref(),
            &lookup,
            &outage,
        ))
        .or_value_error()?;
    org_fields(&token)
}

pub(super) fn safe_fetch_outcome(input: &Input) -> HelperResult {
    let Some(failure) = input.failure else {
        return Err(invalid("the call needs a failure"));
    };
    let runtime = runtime()?;
    let re_raised = match failure {
        Failure::Proxy => runtime
            .block_on(safe_fetch("context", async {
                Err::<(), ProxyException>(proxy_denial())
            }))
            .is_err(),
        Failure::TeamNotFound => runtime
            .block_on(safe_fetch("context", async {
                Err::<(), TeamLookupError>(TeamLookupError::TeamNotFound {
                    team_id: "team-x".to_owned(),
                })
            }))
            .is_err(),
        Failure::NoDatabase => runtime
            .block_on(safe_fetch("context", async {
                Err::<(), TeamLookupError>(TeamLookupError::NoDatabase)
            }))
            .is_err(),
    };
    Ok(if re_raised {
        json!({ "outcome": "re-raised", "class": failure_name(failure) })
    } else {
        json!({ "outcome": "none" })
    })
}

const fn failure_name(failure: Failure) -> &'static str {
    match failure {
        Failure::Proxy => "proxy",
        Failure::TeamNotFound => "team_not_found",
        Failure::NoDatabase => "no_database",
    }
}
