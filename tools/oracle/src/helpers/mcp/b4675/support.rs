use std::collections::HashMap;
use std::fmt::Display;
use std::future::{Future, ready};
use std::sync::Mutex;

use betterlite_db::stored_cipher::DbCipher;
use betterlite_mcp::gateway_authorize::reply::RequestView;
use betterlite_mcp::gateway_dcr::contracts::GATEWAY_DCR_CLIENT_ID_PREFIX;
use betterlite_mcp::gateway_dcr::models::GatewayDcrClient;
use betterlite_mcp::gateway_dcr::ports::{ConsentTeam, ReloadUserFailure, VendorCredentialState};
use betterlite_mcp::gateway_dcr::sealed::seal;
use betterlite_mcp::gateway_dcr::single_use::{
    ClaimAuthority, ClaimBackend, ClaimBackendError, ClaimClock, MemoryClaimStore, SingleUseGuard,
};
use betterlite_mcp::registry::servers::{RegistrySettings, ServerRegistry};
use betterlite_mcp::server_record::MCPServer;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces, StoredCipher};
use secrecy::SecretString;
use serde_json::Value;

use super::answers::Answers;
use super::input::{ClientSeed, Step, TeamsFailure, VendorKind};

pub(super) const BASE: &str = "https://proxy.test:8443";

const SALT: &str = "b4675-oracle-salt";

pub(super) struct ZeroClock;

impl ClaimClock for ZeroClock {
    fn now_seconds(&self) -> i64 {
        0
    }
}

pub(super) struct NoBackend;

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

pub(super) fn guard() -> SingleUseGuard<NoBackend> {
    SingleUseGuard::from(ClaimAuthority::Local(MemoryClaimStore {
        clock: Box::new(ZeroClock),
        entries: Mutex::new(HashMap::new()),
    }))
}

pub(super) fn cipher() -> DbCipher {
    StoredCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from(SALT)),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

pub(super) fn client_ids(seeds: &[ClientSeed], cipher: &DbCipher) -> Vec<String> {
    seeds
        .iter()
        .map(|seed| {
            let client = GatewayDcrClient {
                redirect_uris: seed.redirect_uris.clone(),
                iat: 0,
            };
            seal(GATEWAY_DCR_CLIENT_ID_PREFIX, &client, cipher).unwrap_or_default()
        })
        .collect()
}

pub(super) fn registry(servers: &[Value]) -> Result<ServerRegistry, Box<HelperError>> {
    let records = servers
        .iter()
        .map(|raw| {
            serde_json::from_value::<MCPServer>(raw.clone()).map_err(|error| failure(&error))
        })
        .collect::<Result<Vec<_>, Box<HelperError>>>()?;
    Ok(ServerRegistry::new(
        RegistrySettings {
            public_mcp_servers: Vec::new(),
            public_hub_strict: false,
            internal_ip_ranges: None,
        },
        Vec::new(),
        records,
    ))
}

pub(super) fn answers(step: &Step) -> Answers {
    let vendor = match step.vendor {
        VendorKind::Present => VendorCredentialState::Present,
        VendorKind::Absent => VendorCredentialState::Absent,
        VendorKind::Unavailable => VendorCredentialState::Unavailable,
    };
    let teams = match step.teams_failure {
        Some(TeamsFailure::Unresolvable) => Err(ReloadUserFailure::Unresolvable),
        Some(TeamsFailure::Unavailable) => Err(ReloadUserFailure::Unavailable),
        Some(TeamsFailure::Faulted) => Err(ReloadUserFailure::Faulted),
        Some(TeamsFailure::NoActiveKey) => Err(ReloadUserFailure::NoActiveKey),
        None => Ok(step
            .teams
            .iter()
            .map(|team| ConsentTeam {
                team_id: team.team_id.clone(),
                team_alias: team.team_alias.clone(),
            })
            .collect()),
    };
    Answers {
        vendor,
        reachable: step.reachable,
        teams,
    }
}

pub(super) fn view(step: &Step, cookie: Option<String>) -> RequestView {
    let cookies = cookie
        .and_then(|pair| {
            pair.split_once('=')
                .map(|(name, value)| (name.to_owned(), value.to_owned()))
        })
        .into_iter()
        .collect();
    RequestView {
        base_url: BASE.to_owned(),
        path: step.path.clone(),
        query: step.query.clone(),
        cookies,
    }
}

pub(super) fn failure<E>(error: &E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "HelperFailure".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
