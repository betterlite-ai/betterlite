use betterlite_auth::auth_identity::records::Principal;
use betterlite_core::api::request_principal_metadata::PrincipalFacts;
use betterlite_core::api::request_principal_metadata::principal::logged_api_key;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use crate::organization_create::caller::OrgCaller;

// user_api_key_dict: the identity the auth dependency publishes plus the key, team and org facts these reports read
#[derive(Clone)]
pub(super) struct ReportCaller {
    pub(super) org_caller: OrgCaller,
    pub(super) api_key: Option<SecretString>,
    pub(super) team_id: Option<String>,
    pub(super) org_id: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct KeyReportView {
    api_key: Option<String>,
    is_session_token: bool,
    key_alias: Option<String>,
    team_id: Option<String>,
    org_id: Option<String>,
}

#[derive(Serialize)]
struct LoggedKeyFacts<'key> {
    api_key: Option<&'key str>,
    is_session_token: bool,
    key_alias: Option<&'key str>,
}

impl ReportCaller {
    pub(super) fn from_principal(principal: &Principal) -> Result<Self, serde_json::Error> {
        let org_caller = OrgCaller::from_principal(principal)?;
        let Some(key) = principal.source_key.0.as_ref() else {
            return Ok(Self {
                org_caller,
                api_key: None,
                team_id: principal.teams.first().map(|team| team.id.clone()),
                org_id: principal
                    .organization
                    .as_ref()
                    .map(|organization| organization.id.clone()),
            });
        };
        let view = serde_json::from_value::<KeyReportView>(serde_json::to_value(key)?)?;
        let facts: PrincipalFacts =
            serde_json::from_value(serde_json::to_value(LoggedKeyFacts {
                api_key: view.api_key.as_deref(),
                is_session_token: view.is_session_token,
                key_alias: view.key_alias.as_deref(),
            })?)?;
        Ok(Self {
            org_caller,
            api_key: logged_api_key(&facts).map(SecretString::from),
            team_id: view.team_id,
            org_id: view.org_id,
        })
    }
}
