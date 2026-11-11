use std::time::Duration;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use num_traits::ToPrimitive as _;
use secrecy::SecretString;
use thiserror::Error;

use crate::pkce_login::callback::{CONNECTION_TIMEOUT, CallbackError, LoopbackServer};
use crate::pkce_login::discovery::discover_cli_auth;
use crate::pkce_login::models::{CallbackOutcome, PkceCredential, PkceFailure};
use crate::pkce_login::oauth::{
    RedeemCodeRequest, authorize_url, pkce_pair, redeem_code, register_client,
};
use crate::pkce_login::{Clock, Entropy, Http, WallClock};
use crate::token_records::record::CliTokenRecord;

use super::contracts::CliTokenData;
use super::notice::MessageSink;

pub const LOGIN_TIMEOUT_SECONDS: u64 = 300;

#[derive(Debug, Error)]
pub enum LoginError {
    #[error(transparent)]
    Pkce(#[from] PkceFailure),
    #[error("cannot start the local sign-in listener: {source}")]
    Listener {
        #[source]
        source: CallbackError,
    },
}

pub struct LoginRequest<'login, H, C, W, E> {
    pub base_url: &'login str,
    pub http: &'login H,
    pub open_browser: MessageSink<'login>,
    pub echo: MessageSink<'login>,
    pub entropy: &'login mut E,
    pub clock: &'login C,
    pub wall_clock: &'login W,
    pub timeout: Duration,
}

pub fn run_pkce_login<H, C, W, E>(
    request: LoginRequest<'_, H, C, W, E>,
) -> Result<PkceCredential, LoginError>
where
    H: Http,
    C: Clock,
    W: WallClock,
    E: Entropy,
{
    let LoginRequest {
        base_url,
        http,
        open_browser,
        echo,
        entropy,
        clock,
        wall_clock,
        timeout,
    } = request;
    let contract = discover_cli_auth(base_url, http)?;
    let state = random_state(entropy)?;
    let (verifier, challenge) = pkce_pair(entropy)?;
    let mut server = LoopbackServer::bind(&state, CONNECTION_TIMEOUT)
        .map_err(|source| LoginError::Listener { source })?;
    let redirect_uri = server.redirect_uri();
    let client_id = register_client(&contract, &redirect_uri, http)?;
    let url = authorize_url(&contract, &client_id, &redirect_uri, &state, &challenge);
    (echo)(&format!("Opening browser to: {url}"));
    (echo)("Approve the sign-in in your browser. Waiting...");
    (open_browser)(&url);
    match server.wait(timeout, clock)? {
        CallbackOutcome::Denied(denied) => Err(LoginError::Pkce(PkceFailure::Protocol(format!(
            "sign-in was not approved ({}): {}",
            denied.error,
            denied
                .description
                .unwrap_or_else(|| "no details".to_owned())
        )))),
        CallbackOutcome::Code(code) => {
            let redeem = RedeemCodeRequest {
                client_id,
                redirect_uri,
                code: code.code,
                verifier,
            };
            redeem_code(&contract, &redeem, http, wall_clock).map_err(LoginError::Pkce)
        }
    }
}

fn random_state<E>(entropy: &mut E) -> Result<String, PkceFailure>
where
    E: Entropy,
{
    let mut bytes = [0_u8; 32];
    entropy.fill(&mut bytes)?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

#[must_use]
pub fn pkce_token_record(
    base_url: &str,
    credential: &PkceCredential,
    clock: &dyn WallClock,
) -> CliTokenData {
    CliTokenRecord {
        base_url: base_url.trim_end_matches('/').to_owned(),
        key: Some(credential.access_token.clone()),
        user_id: credential
            .user_id
            .clone()
            .unwrap_or_else(|| "cli-user".to_owned()),
        user_email: "unknown".to_owned(),
        user_role: "cli".to_owned(),
        auth_header_name: "Authorization".to_owned(),
        jwt_token: SecretString::default(),
        timestamp: clock.now_seconds().to_f64().unwrap_or_default(),
        expires_at: Some(credential.expires_at.to_f64().unwrap_or_default()),
        refresh_token: Some(credential.refresh_token.clone()),
        client_id: Some(credential.client_id.clone()),
        token_endpoint: Some(credential.token_endpoint.clone()),
        revocation_endpoint: Some(credential.revocation_endpoint.clone()),
        resource: Some(credential.resource.clone()),
        team_id: credential.team_id.clone(),
    }
}
