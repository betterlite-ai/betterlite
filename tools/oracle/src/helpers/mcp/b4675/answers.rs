use std::collections::VecDeque;
use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};

use betterlite_mcp::gateway_authorize::reply::{FlowTokenSource, RenderConsentPage};
use betterlite_mcp::gateway_dcr::ports::{
    ConsentTeam, LookupConsentTeams, LookupServerReachability, LookupVendorCredential,
    ReloadUserFailure, VendorCredentialState,
};

pub(super) struct Answers {
    pub(super) vendor: VendorCredentialState,
    pub(super) reachable: bool,
    pub(super) teams: Result<Vec<ConsentTeam>, ReloadUserFailure>,
}

impl LookupVendorCredential for Answers {
    fn lookup(
        &self,
        _user_id: &str,
        _server_id: &str,
    ) -> impl Future<Output = VendorCredentialState> + Send {
        ready(self.vendor)
    }
}

impl LookupServerReachability for Answers {
    fn reachable(&self, _user_id: &str, _server_id: &str) -> impl Future<Output = bool> + Send {
        ready(self.reachable)
    }
}

impl LookupConsentTeams for Answers {
    fn consent_teams(
        &self,
        _user_id: &str,
    ) -> impl Future<Output = Result<Vec<ConsentTeam>, ReloadUserFailure>> + Send {
        ready(self.teams.clone())
    }
}

impl RenderConsentPage for Answers {
    fn render(
        &self,
        client_origin: &str,
        user_id: &str,
        teams: &[(String, String)],
        flow_handle: &str,
        complete_url: &str,
    ) -> String {
        let named: Vec<String> = teams
            .iter()
            .map(|(id, alias)| format!("[\"{id}\",\"{alias}\"]"))
            .collect();
        format!(
            "{{\"client_origin\":\"{client_origin}\",\"complete_url\":\"{complete_url}\",\"flow_handle\":\"{flow_handle}\",\"teams\":[{}],\"user_id\":\"{user_id}\"}}",
            named.join(",")
        )
    }
}

pub(super) struct ScriptedTokens {
    values: Mutex<VecDeque<String>>,
}

impl ScriptedTokens {
    pub(super) fn new(values: &[String]) -> Self {
        Self {
            values: Mutex::new(values.iter().cloned().collect()),
        }
    }
}

impl FlowTokenSource for ScriptedTokens {
    fn fresh_token(&self) -> String {
        let mut values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        if values.is_empty() {
            return String::new();
        }
        values.pop_front().unwrap_or_default()
    }
}
