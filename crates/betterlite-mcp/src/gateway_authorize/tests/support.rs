#![cfg(test)]

use std::collections::HashMap;
use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};

use betterlite_db::stored_cipher::DbCipher;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces, StoredCipher};
use secrecy::SecretString;

use crate::gateway_authorize::reply::{FlowTokenSource, RenderConsentPage, RequestView};
use crate::gateway_authorize::url_parts::flow_cookie_name;
use crate::gateway_dcr::contracts::GATEWAY_DCR_CLIENT_ID_PREFIX;
use crate::gateway_dcr::models::{ConnectFlow, GatewayDcrClient};
use crate::gateway_dcr::ports::{
    ConsentTeam, LookupConsentTeams, LookupServerReachability, LookupVendorCredential,
    ReloadUserFailure, VendorCredentialState,
};
use crate::gateway_dcr::sealed::seal;
use crate::gateway_dcr::single_use::{
    ClaimAuthority, ClaimBackend, ClaimBackendError, ClaimClock, MemoryClaimStore, SingleUseGuard,
};
use crate::registry::servers::{RegistrySettings, ServerRegistry};
use crate::server_record::MCPServer;

pub(crate) const BASE: &str = "https://proxy.test:8443";
pub(crate) const CALLBACK: &str = "http://127.0.0.1:8080/callback";

pub(crate) fn cipher() -> DbCipher {
    StoredCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from("b4675-oracle-salt")),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

pub(crate) fn client_id(redirect_uris: &[&str]) -> String {
    let client = GatewayDcrClient {
        redirect_uris: redirect_uris.iter().map(|uri| (*uri).to_owned()).collect(),
        iat: 0,
    };
    seal(GATEWAY_DCR_CLIENT_ID_PREFIX, &client, &cipher()).unwrap()
}

pub(crate) fn flow() -> ConnectFlow {
    ConnectFlow {
        user_id: "alice".to_owned(),
        client_id: "llm_dcrc_client".to_owned(),
        redirect_uri: CALLBACK.to_owned(),
        state: "st-1".to_owned(),
        code_challenge: "challenge-1".to_owned(),
        jti: "jti-1".to_owned(),
        exp: 1_000,
        resource_server_id: None,
        audience: None,
    }
}

pub(crate) fn view(path: &str, query: Option<&str>, cookies: &[(&str, &str)]) -> RequestView {
    RequestView {
        base_url: BASE.to_owned(),
        path: path.to_owned(),
        query: query.map(str::to_owned),
        cookies: cookies
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
    }
}

pub(crate) fn sealed_view(flow: &ConnectFlow, handle: &str) -> RequestView {
    let sealed = seal("", flow, &cipher()).unwrap();
    let name = flow_cookie_name(handle);
    view(
        "/authorize/complete",
        None,
        &[("mcp_connect_flow_other", "junk"), (name.as_str(), sealed.as_str())],
    )
}

pub(crate) struct ScriptedTokens {
    pub(crate) values: Mutex<Vec<String>>,
}

impl ScriptedTokens {
    pub(crate) fn new(values: &[&str]) -> Self {
        Self {
            values: Mutex::new(values.iter().map(|value| (*value).to_owned()).collect()),
        }
    }
}

impl FlowTokenSource for ScriptedTokens {
    fn fresh_token(&self) -> String {
        let mut values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        if values.is_empty() {
            String::new()
        } else {
            values.remove(0)
        }
    }
}

pub(crate) struct Page;

impl RenderConsentPage for Page {
    fn render(
        &self,
        client_origin: &str,
        user_id: &str,
        teams: &[(String, String)],
        flow_handle: &str,
        complete_url: &str,
    ) -> String {
        format!("{client_origin}|{user_id}|{teams:?}|{flow_handle}|{complete_url}")
    }
}

pub(crate) struct Vendor(pub(crate) VendorCredentialState);

impl LookupVendorCredential for Vendor {
    fn lookup(
        &self,
        _user_id: &str,
        _server_id: &str,
    ) -> impl Future<Output = VendorCredentialState> + Send {
        ready(self.0)
    }
}

pub(crate) struct Reach(pub(crate) bool);

impl LookupServerReachability for Reach {
    fn reachable(&self, _user_id: &str, _server_id: &str) -> impl Future<Output = bool> + Send {
        ready(self.0)
    }
}

pub(crate) struct Teams(pub(crate) Result<Vec<ConsentTeam>, ReloadUserFailure>);

impl LookupConsentTeams for Teams {
    fn consent_teams(
        &self,
        _user_id: &str,
    ) -> impl Future<Output = Result<Vec<ConsentTeam>, ReloadUserFailure>> + Send {
        ready(self.0.clone())
    }
}

pub(crate) struct ZeroClock;

impl ClaimClock for ZeroClock {
    fn now_seconds(&self) -> i64 {
        0
    }
}

pub(crate) struct NoBackend;

impl ClaimBackend for NoBackend {
    fn increment(
        &self,
        _key: &str,
        _ttl_seconds: i64,
    ) -> impl Future<Output = Result<u64, ClaimBackendError>> + Send {
        ready(Err(ClaimBackendError))
    }

    fn read(
        &self,
        _key: &str,
    ) -> impl Future<Output = Result<Option<u64>, ClaimBackendError>> + Send {
        ready(Err(ClaimBackendError))
    }
}

pub(crate) fn guard() -> SingleUseGuard<NoBackend> {
    SingleUseGuard::from(ClaimAuthority::Local(MemoryClaimStore {
        clock: Box::new(ZeroClock),
        entries: Mutex::new(HashMap::new()),
    }))
}

pub(crate) fn registry(servers: &[&str]) -> ServerRegistry {
    let records = servers
        .iter()
        .map(|json| serde_json::from_str::<MCPServer>(json).unwrap())
        .collect();
    ServerRegistry::new(
        RegistrySettings {
            public_mcp_servers: Vec::new(),
            public_hub_strict: false,
            internal_ip_ranges: None,
        },
        Vec::new(),
        records,
    )
}

pub(crate) const OAUTH_SERVER: &str = r#"{"server_id":"srv-a","name":"alpha","transport":"sse","auth_type":"oauth2","oauth2_flow":"authorization_code"}"#;
pub(crate) const M2M_SERVER: &str = r#"{"server_id":"srv-b","name":"beta","transport":"sse","auth_type":"oauth2","oauth2_flow":"client_credentials"}"#;
pub(crate) const DELEGATED_SERVER: &str = r#"{"server_id":"srv-c","name":"gamma","transport":"sse","auth_type":"oauth2","oauth2_flow":"authorization_code","delegate_auth_to_upstream":true}"#;
