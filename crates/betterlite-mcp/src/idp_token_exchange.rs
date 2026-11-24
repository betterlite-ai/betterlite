mod tests;

use std::future::Future;

use betterlite_auth::resolved_key::credential::is_jwt;
use betterlite_db::database_availability::chain::Chain;
use betterlite_db::database_availability::classification::is_permanent_fault;
use betterlite_db::database_faults::DbError;

use crate::gateway_dcr::subject_token::{
    ExchangeSubjectToken, RefusalCode, SubjectIdentity, SubjectTokenRefusal,
};

pub const EXCHANGE_ROUTE: &str = "/token";
pub const REJECTED_SUBJECT_TOKEN: &str = "subject_token was rejected by the gateway's JWT auth";
pub const SUBJECT_CHECK_UNAVAILABLE: &str = "the gateway could not verify subject_token because its identity provider or database is unavailable; retry";
pub const SUBJECT_CHECK_FAULTED: &str = "the gateway could not verify subject_token because its database reported a fault that is not a transient outage; retrying will not help until the gateway deployment is repaired";

#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display)]
#[strum(serialize_all = "snake_case")]
pub enum GatewayOutage {
    Retryable,
    Faulted,
}

// clippy's struct_excessive_bools allows two bools per struct, and these three gates are independent
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrerequisiteSwitch(pub bool);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenExchangePrerequisites {
    pub jwt_auth_enabled: PrerequisiteSwitch,
    pub has_database: PrerequisiteSwitch,
    pub maps_jwts_to_virtual_keys: PrerequisiteSwitch,
}

impl TokenExchangePrerequisites {
    #[must_use]
    pub fn refusal(self) -> Option<SubjectTokenRefusal> {
        if !self.jwt_auth_enabled.0 {
            return Some(SubjectTokenRefusal {
                error: RefusalCode::UnsupportedGrantType,
                description:
                    "JWT auth is not enabled on this gateway, so it cannot exchange IdP tokens"
                        .to_owned(),
            });
        }
        if !self.has_database.0 {
            return Some(SubjectTokenRefusal {
                error: RefusalCode::UnsupportedGrantType,
                description: "this gateway has no database, so it cannot exchange IdP tokens"
                    .to_owned(),
            });
        }
        if self.maps_jwts_to_virtual_keys.0 {
            return Some(SubjectTokenRefusal {
                error: RefusalCode::UnsupportedGrantType,
                description: "this gateway maps IdP tokens to virtual keys, which the exchange does not serve"
                    .to_owned(),
            });
        }
        None
    }
}

pub trait TokenExchangeInputs {
    fn prerequisites(&self) -> TokenExchangePrerequisites;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizedSubject {
    pub user_id: Option<String>,
    pub team_id: Option<String>,
    pub agent_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DenialSource {
    Http { status: u16 },
    Proxy { code: String },
    Plain,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubjectTokenDenial {
    pub source: DenialSource,
    pub reason: String,
    pub database_cause: Option<DbError>,
}

pub trait AuthorizeSubjectToken {
    type Context;

    // an async fn in a trait cannot promise Send, which ExchangeSubjectToken requires
    fn authorize(
        &self,
        subject_token: &str,
        context: &Self::Context,
    ) -> impl Future<Output = Result<AuthorizedSubject, SubjectTokenDenial>> + Send;
}

pub struct IdpTokenExchange<I, A> {
    pub inputs: I,
    pub authorizer: A,
}

impl<I, A> ExchangeSubjectToken<A::Context> for IdpTokenExchange<I, A>
where
    I: TokenExchangeInputs + Sync,
    A: AuthorizeSubjectToken + Sync,
    A::Context: Sync,
{
    fn exchange(
        &self,
        subject_token: &str,
        context: &A::Context,
    ) -> impl Future<Output = Result<SubjectIdentity, SubjectTokenRefusal>> + Send {
        identity_from_subject_token(subject_token, context, self)
    }
}

pub async fn identity_from_subject_token<I, A>(
    subject_token: &str,
    context: &A::Context,
    exchange: &IdpTokenExchange<I, A>,
) -> Result<SubjectIdentity, SubjectTokenRefusal>
where
    I: TokenExchangeInputs + Sync,
    A: AuthorizeSubjectToken + Sync,
    A::Context: Sync,
{
    if let Some(refusal) = exchange.inputs.prerequisites().refusal() {
        return Err(refusal);
    }
    if !is_jwt(Some(subject_token)) {
        return Err(SubjectTokenRefusal {
            error: RefusalCode::InvalidRequest,
            description: "subject_token is not a JWT".to_owned(),
        });
    }
    let authorized = exchange
        .authorizer
        .authorize(subject_token, context)
        .await
        .map_err(|denial| refusal_for(&denial))?;
    if authorized.agent_id.is_some() {
        return Err(SubjectTokenRefusal {
            error: RefusalCode::InvalidRequest,
            description:
                "Agent tokens require direct JWT authentication; this exchange supports users only"
                    .to_owned(),
        });
    }
    let Some(user_id) = authorized.user_id.filter(|user_id| !user_id.is_empty()) else {
        return Err(SubjectTokenRefusal {
            error: RefusalCode::InvalidRequest,
            description: "subject_token names no user the gateway knows".to_owned(),
        });
    };
    Ok(SubjectIdentity {
        user_id,
        team_id: authorized.team_id,
    })
}

fn refusal_for(denial: &SubjectTokenDenial) -> SubjectTokenRefusal {
    let Some(outage) = gateway_outage(denial) else {
        tracing::warn!("token exchange refused a subject_token: {}", denial.reason);
        return SubjectTokenRefusal {
            error: RefusalCode::InvalidRequest,
            description: REJECTED_SUBJECT_TOKEN.to_owned(),
        };
    };
    tracing::error!(
        "token exchange could not verify a subject_token, {outage}: {}",
        denial.reason
    );
    SubjectTokenRefusal {
        error: RefusalCode::TemporarilyUnavailable,
        description: check_unavailable_description(outage).to_owned(),
    }
}

fn gateway_outage(denial: &SubjectTokenDenial) -> Option<GatewayOutage> {
    if let Some(cause) = &denial.database_cause {
        let chain = Chain::from_error(cause);
        if let Some(fault) = chain.find_outage() {
            return Some(if is_permanent_fault(fault) {
                GatewayOutage::Faulted
            } else {
                GatewayOutage::Retryable
            });
        }
    }
    is_server_error(&denial.source).then_some(GatewayOutage::Retryable)
}

fn is_server_error(source: &DenialSource) -> bool {
    match source {
        DenialSource::Http { status } => *status >= 500,
        DenialSource::Proxy { code } => proxy_code_is_server_error(code),
        DenialSource::Plain => false,
    }
}

fn proxy_code_is_server_error(code: &str) -> bool {
    let digits = code.trim_start_matches('0');
    if digits.is_empty() || !code.bytes().all(|byte| byte.is_ascii_digit()) {
        return false;
    }
    if let Ok(value) = digits.parse::<u16>() {
        return value >= 500;
    }
    // a code past u16's range is past every status this check compares against
    true
}

const fn check_unavailable_description(outage: GatewayOutage) -> &'static str {
    match outage {
        GatewayOutage::Retryable => SUBJECT_CHECK_UNAVAILABLE,
        GatewayOutage::Faulted => SUBJECT_CHECK_FAULTED,
    }
}
