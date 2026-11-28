use std::collections::HashMap;
use std::future::{Future, ready};
use std::num::NonZeroU64;
use std::sync::Mutex;

use aws_lc_rs::error::Unspecified;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_mcp::gateway_dcr::contracts::GATEWAY_DCR_CLIENT_ID_PREFIX;
use betterlite_mcp::gateway_dcr::models::GatewayDcrClient;
use betterlite_mcp::gateway_dcr::ports::{
    MintProxyCredential, MintedProxyCredential, ProxyCredentialMintFailure, ReloadUser,
    ReloadUserFailure,
};
use betterlite_mcp::gateway_dcr::sealed::seal;
use betterlite_mcp::gateway_dcr::single_use::{
    ClaimAuthority, ClaimBackend, ClaimBackendError, ClaimClock, MemoryClaimStore, SingleUseGuard,
};
use betterlite_mcp::gateway_dcr::subject_token::{
    ExchangeSubjectToken, RefusalCode, SubjectIdentity, SubjectTokenRefusal,
};
use betterlite_mcp::gateway_token::support::{SessionEntropy, SigningSetup};
use betterlite_mcp::registry::servers::{RegistrySettings, ServerRegistry};
use betterlite_mcp::server_record::MCPServer;
use betterlite_mcp::session_credentials::session_keys_from_master_key;
use betterlite_mcp::types::SessionSigningKeys;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces, StoredCipher};
use secrecy::SecretString;
use serde_json::Value;

use super::input::{
    ClientSeed, ExchangeAnswer, ExchangeFailure, MintAnswer, MintFailureKind, RefusalKind,
    ReloadKind, Step,
};

const MASTER: &str = "sk-b4676-oracle-master-key";
const SALT: &str = "b4676-oracle-salt";
const NO_EXCHANGE_ANSWER: &str = "the case lists no exchange answer";

pub(super) const FIXED_JTI: [u8; 16] = [7_u8; 16];

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub(super) struct CaseDefect(pub(super) String);

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

pub(super) fn cipher() -> DbCipher {
    let settings = CipherSettings {
        signing_key: Some(SecretString::from(SALT)),
        algorithm: Algorithm::XSalsa20Poly1305,
    };
    StoredCipher {
        settings,
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

pub(super) fn guard() -> SingleUseGuard<NoBackend> {
    let store = MemoryClaimStore {
        clock: Box::new(ZeroClock),
        entries: Mutex::new(HashMap::new()),
    };
    SingleUseGuard::from(ClaimAuthority::Local(store))
}

pub(super) fn client_ids(
    seeds: &[ClientSeed],
    cipher: &DbCipher,
) -> Result<Vec<String>, Box<HelperError>> {
    let mut clients = Vec::with_capacity(seeds.len());
    for seed in seeds {
        let client = GatewayDcrClient {
            redirect_uris: seed.redirect_uris.clone(),
            iat: 0,
        };
        clients.push(seal(GATEWAY_DCR_CLIENT_ID_PREFIX, &client, cipher).or_value_error()?);
    }
    Ok(clients)
}

pub(super) fn registry(servers: &[Value]) -> Result<ServerRegistry, Box<HelperError>> {
    let mut records = Vec::with_capacity(servers.len());
    for raw in servers {
        records.push(serde_json::from_value::<MCPServer>(raw.clone()).or_config_error()?);
    }
    Ok(ServerRegistry::new(
        registry_settings(),
        Vec::new(),
        records,
    ))
}

fn registry_settings() -> RegistrySettings {
    RegistrySettings {
        public_mcp_servers: Vec::new(),
        public_hub_strict: false,
        internal_ip_ranges: None,
    }
}

pub(super) fn present_setup() -> Result<SigningSetup, Box<HelperError>> {
    let master = SecretString::from(MASTER);
    let keys = session_keys_from_master_key(&master).or_config_error()?;
    Ok(SigningSetup::Ready(SessionSigningKeys::Symmetric(keys)))
}

pub(super) struct FixedEntropy;

impl SessionEntropy for FixedEntropy {
    fn jti(&self) -> Result<[u8; 16], Unspecified> {
        Ok(FIXED_JTI)
    }
}

pub(super) struct Answers {
    reload: Option<ReloadUserFailure>,
    minted: Option<MintedProxyCredential>,
    mint_failure: Option<ProxyCredentialMintFailure>,
    identity: Option<SubjectIdentity>,
    refusal: Option<SubjectTokenRefusal>,
}

impl Answers {
    pub(super) fn new(step: &Step) -> Result<Self, Box<HelperError>> {
        let minted = match &step.mint {
            Some(answer) => Some(minted_credential(answer)?),
            None => None,
        };
        Ok(Self {
            reload: step.reload.map(reload_failure),
            minted,
            mint_failure: step.mint_failure.map(mint_failure),
            identity: step.exchange.as_ref().map(subject_identity),
            refusal: step.exchange_failure.as_ref().map(subject_refusal),
        })
    }
}

impl ReloadUser for Answers {
    fn reload_user(
        &self,
        _user_id: &str,
    ) -> impl Future<Output = Option<ReloadUserFailure>> + Send {
        ready(self.reload)
    }
}

impl MintProxyCredential for Answers {
    fn mint(
        &self,
        _user_id: &str,
        _team_id: Option<&str>,
    ) -> impl Future<Output = Result<MintedProxyCredential, ProxyCredentialMintFailure>> + Send
    {
        let answer = match (self.mint_failure, &self.minted) {
            (Some(failure), _) => Err(failure),
            (None, Some(minted)) => Ok(minted.clone()),
            (None, None) => Err(ProxyCredentialMintFailure::Unresolvable),
        };
        ready(answer)
    }
}

impl ExchangeSubjectToken<()> for Answers {
    fn exchange(
        &self,
        _subject_token: &str,
        _context: &(),
    ) -> impl Future<Output = Result<SubjectIdentity, SubjectTokenRefusal>> + Send {
        let answer = match (&self.refusal, &self.identity) {
            (Some(refusal), _) => Err(refusal.clone()),
            (None, Some(identity)) => Ok(identity.clone()),
            (None, None) => Err(SubjectTokenRefusal {
                error: RefusalCode::InvalidRequest,
                description: NO_EXCHANGE_ANSWER.to_owned(),
            }),
        };
        ready(answer)
    }
}

fn minted_credential(answer: &MintAnswer) -> Result<MintedProxyCredential, Box<HelperError>> {
    let expires_in = u64::try_from(answer.expires_in)
        .ok()
        .and_then(NonZeroU64::new)
        .ok_or_else(|| CaseDefect("the mint answer needs a positive expires_in".to_owned()))
        .or_value_error()?;
    Ok(MintedProxyCredential {
        key: SecretString::from(answer.key.clone()),
        expires_in,
        user_id: answer.user_id.clone(),
        team_id: answer.team_id.clone(),
    })
}

const fn reload_failure(kind: ReloadKind) -> ReloadUserFailure {
    match kind {
        ReloadKind::Unresolvable => ReloadUserFailure::Unresolvable,
        ReloadKind::Unavailable => ReloadUserFailure::Unavailable,
        ReloadKind::Faulted => ReloadUserFailure::Faulted,
        ReloadKind::NoActiveKey => ReloadUserFailure::NoActiveKey,
    }
}

const fn mint_failure(kind: MintFailureKind) -> ProxyCredentialMintFailure {
    match kind {
        MintFailureKind::Unresolvable => ProxyCredentialMintFailure::Unresolvable,
        MintFailureKind::Unavailable => ProxyCredentialMintFailure::Unavailable,
        MintFailureKind::Faulted => ProxyCredentialMintFailure::Faulted,
        MintFailureKind::NoActiveKey => ProxyCredentialMintFailure::NoActiveKey,
        MintFailureKind::NotAMember => ProxyCredentialMintFailure::NotAMember,
        MintFailureKind::TeamRequired => ProxyCredentialMintFailure::TeamRequired,
    }
}

fn subject_identity(answer: &ExchangeAnswer) -> SubjectIdentity {
    SubjectIdentity {
        user_id: answer.user_id.clone(),
        team_id: answer.team_id.clone(),
    }
}

fn subject_refusal(failure: &ExchangeFailure) -> SubjectTokenRefusal {
    SubjectTokenRefusal {
        error: refusal_code(failure.error),
        description: failure.description.clone(),
    }
}

const fn refusal_code(kind: RefusalKind) -> RefusalCode {
    match kind {
        RefusalKind::UnsupportedGrantType => RefusalCode::UnsupportedGrantType,
        RefusalKind::InvalidRequest => RefusalCode::InvalidRequest,
        RefusalKind::TemporarilyUnavailable => RefusalCode::TemporarilyUnavailable,
    }
}
