mod tests;

use std::future::{Future, ready};
use std::num::NonZeroU64;

use secrecy::SecretString;

use crate::gateway_dcr::subject_token::{
    ExchangeSubjectToken, RefusalCode, SubjectIdentity, SubjectTokenRefusal,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReloadUserFailure {
    Unresolvable,
    Unavailable,
    Faulted,
    NoActiveKey,
}

pub trait ReloadUser: Send + Sync {
    fn reload_user(&self, user_id: &str) -> impl Future<Output = Option<ReloadUserFailure>> + Send;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VendorCredentialState {
    Present,
    Absent,
    Unavailable,
}

#[derive(Clone, Debug)]
pub struct MintedProxyCredential {
    pub key: SecretString,
    pub expires_in: NonZeroU64,
    pub user_id: String,
    pub team_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProxyCredentialMintFailure {
    Unresolvable,
    Unavailable,
    Faulted,
    NoActiveKey,
    NotAMember,
    TeamRequired,
}

pub trait MintProxyCredential: Send + Sync {
    fn mint(
        &self,
        user_id: &str,
        team_id: Option<&str>,
    ) -> impl Future<Output = Result<MintedProxyCredential, ProxyCredentialMintFailure>> + Send;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsentTeam {
    pub team_id: String,
    pub team_alias: Option<String>,
}

pub trait LookupVendorCredential: Send + Sync {
    fn lookup(
        &self,
        user_id: &str,
        server_id: &str,
    ) -> impl Future<Output = VendorCredentialState> + Send;
}

pub trait LookupServerReachability: Send + Sync {
    fn reachable(&self, user_id: &str, server_id: &str) -> impl Future<Output = bool> + Send;
}

pub trait LookupConsentTeams: Send + Sync {
    fn consent_teams(
        &self,
        user_id: &str,
    ) -> impl Future<Output = Result<Vec<ConsentTeam>, ReloadUserFailure>> + Send;
}

pub struct RefuseProxyCredential;
impl MintProxyCredential for RefuseProxyCredential {
    fn mint(
        &self,
        _user_id: &str,
        _team_id: Option<&str>,
    ) -> impl Future<Output = Result<MintedProxyCredential, ProxyCredentialMintFailure>> + Send
    {
        ready(Err(ProxyCredentialMintFailure::Unresolvable))
    }
}

pub struct RefuseSubjectToken;
impl<Context> ExchangeSubjectToken<Context> for RefuseSubjectToken {
    fn exchange(
        &self,
        _subject_token: &str,
        _context: &Context,
    ) -> impl Future<Output = Result<SubjectIdentity, SubjectTokenRefusal>> + Send {
        ready(Err(SubjectTokenRefusal {
            error: RefusalCode::UnsupportedGrantType,
            description: SUBJECT_REFUSAL_DESCRIPTION.to_owned(),
        }))
    }
}

pub struct UnavailableVendorCredential;
impl LookupVendorCredential for UnavailableVendorCredential {
    fn lookup(
        &self,
        _user_id: &str,
        _server_id: &str,
    ) -> impl Future<Output = VendorCredentialState> + Send {
        ready(VendorCredentialState::Unavailable)
    }
}

pub struct UnreachableServer;

impl LookupServerReachability for UnreachableServer {
    fn reachable(&self, _user_id: &str, _server_id: &str) -> impl Future<Output = bool> + Send {
        ready(false)
    }
}

pub const SUBJECT_REFUSAL_DESCRIPTION: &str =
    "this gateway is not configured to exchange IdP tokens";
