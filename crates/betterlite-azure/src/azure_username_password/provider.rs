use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

use secrecy::SecretString;

use crate::azure_client_secret::authority::{Authority, discover_authority, normalize_authority};
use crate::azure_client_secret::transport::Clock;

use super::authority;
use super::record::AuthenticationRecord;
use super::token;
use super::transport::Transport;
use super::{DEFAULT_AUTHORITY_HOST, UsernamePasswordError, UsernamePasswordInput};

// the azure-core bearer policy spends a token that has five minutes left, and the credential renews it
const REUSE_WINDOW_SECONDS: i64 = 300;
pub const DEPRECATION_WARNING: &str = "UsernamePasswordCredential is deprecated, as it doesn't support multifactor authentication (MFA). For more details, see https://aka.ms/azsdk/identity/mfa.";

#[derive(Clone)]
pub struct AccessToken {
    pub token: SecretString,
    pub expires_on: i64,
    pub token_type: String,
    pub refresh_on: Option<i64>,
    pub record: AuthenticationRecord,
}

// the identity record names the user, and a Debug of the token must not put it in a log line
impl fmt::Debug for AccessToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AccessToken")
            .field("token", &self.token)
            .field("expires_on", &self.expires_on)
            .field("token_type", &self.token_type)
            .field("refresh_on", &self.refresh_on)
            .finish_non_exhaustive()
    }
}

pub struct UsernamePasswordProvider {
    input: UsernamePasswordInput,
    authority_host: String,
    transport: Arc<dyn Transport>,
    clock: Arc<dyn Clock>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    token: Option<AccessToken>,
    authority: Option<Authority>,
}

// the password, the tokens and the user identity stay out of the Debug output
impl fmt::Debug for UsernamePasswordProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UsernamePasswordProvider")
            .field("client_id", &self.input.client_id)
            .field("authority_host", &self.authority_host)
            .field("scope", &self.input.scope)
            .finish_non_exhaustive()
    }
}

impl UsernamePasswordProvider {
    pub fn new(
        input: &UsernamePasswordInput,
        authority_host: Option<&str>,
        transport: Arc<dyn Transport>,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, UsernamePasswordError> {
        // LiteLLM logs whether each argument is not None, so a typed input still reports an empty string as set
        tracing::debug!(
            client_id = %input.client_id,
            username_set = true,
            password_set = true,
            "client_id={}, azure_username=[set=true], azure_password=[set=true]",
            input.client_id
        );
        tracing::warn!(category = "DeprecationWarning", "{DEPRECATION_WARNING}");
        let host = normalize_authority(authority_host.unwrap_or(DEFAULT_AUTHORITY_HOST))?;
        let provider = Self {
            input: input.clone(),
            authority_host: host,
            transport,
            clock,
            state: Mutex::new(State::default()),
        };
        tracing::debug!("token_provider {:?}", provider);
        Ok(provider)
    }

    pub fn token(&self) -> Result<AccessToken, UsernamePasswordError> {
        let now = self.clock.now_unix_seconds();
        {
            let state = self.state()?;
            if let Some(token) = &state.token
                && token.expires_on.saturating_sub(now) >= REUSE_WINDOW_SECONDS
            {
                return Ok(token.clone());
            }
        }
        let authority = self.authority()?;
        let response = token::request_token(&*self.transport, &authority, &self.input)?;
        let expires_on = now
            .checked_add(response.expires_in)
            .ok_or(UsernamePasswordError::ExpiryOverflow)?;
        let refresh_on = match response.refresh_in {
            Some(refresh_in) => Some(
                now.checked_add(refresh_in)
                    .ok_or(UsernamePasswordError::ExpiryOverflow)?,
            ),
            None => response.refresh_on,
        };
        let access = AccessToken {
            token: response.access_token,
            expires_on,
            token_type: response.token_type,
            refresh_on,
            record: response.record,
        };
        let mut state = self.state()?;
        state.token = Some(access.clone());
        drop(state);
        Ok(access)
    }

    // MSAL builds its application, authority included, once per credential and keeps the app for every grant
    fn authority(&self) -> Result<Authority, UsernamePasswordError> {
        {
            let state = self.state()?;
            if let Some(authority) = &state.authority {
                return Ok(authority.clone());
            }
        }
        let bridged = authority::Bridged {
            transport: &*self.transport,
        };
        let authority = discover_authority(
            &bridged,
            &self.authority_host,
            authority::ORGANIZATIONS_TENANT,
            true,
        )?;
        self.state()?.authority = Some(authority.clone());
        Ok(authority)
    }

    fn state(&self) -> Result<MutexGuard<'_, State>, UsernamePasswordError> {
        self.state
            .lock()
            .map_err(|_error| UsernamePasswordError::ProviderCachePoisoned)
    }
}
