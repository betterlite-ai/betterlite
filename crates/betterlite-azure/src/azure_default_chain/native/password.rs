use std::sync::{Arc, Mutex, MutexGuard};

use crate::azure_arc_identity::ArcHost;
use crate::azure_client_secret::authority::{Authority, discover_authority};
use crate::azure_client_secret::transport::Clock;
use crate::azure_username_password::UsernamePasswordInput;
use crate::azure_username_password::authority::Bridged;
use crate::azure_username_password::token::{self, PasswordToken};
use crate::azure_username_password::transport::Transport as PasswordTransport;

use super::super::grant::{ChainToken, Grant, LinkError, UsernamePasswordRecord};
use super::NativeGrants;

const DEFAULT_AUTHORITY_HOST: &str = "login.microsoftonline.com";
const REUSE_WINDOW_SECONDS: i64 = 300;

#[derive(Default)]
pub struct State {
    token: Option<ChainToken>,
    authority: Option<Authority>,
}

// the environment branch keeps the SDK's configured tenant, which the B675 wrapper replaces with organizations
pub struct EnvironmentPasswordGrant<'io> {
    pub input: UsernamePasswordInput,
    pub tenant: String,
    pub authority_host: String,
    pub transport: &'io dyn PasswordTransport,
    pub clock: Arc<dyn Clock>,
    pub state: Mutex<State>,
}

impl EnvironmentPasswordGrant<'_> {
    pub fn from_record<'grants, 'io, H>(
        record: &UsernamePasswordRecord,
        source: &'grants NativeGrants<'io, H>,
    ) -> EnvironmentPasswordGrant<'grants>
    where
        'io: 'grants,
        H: ArcHost,
    {
        EnvironmentPasswordGrant {
            input: UsernamePasswordInput {
                client_id: record.client_id.clone(),
                username: record.username.clone(),
                password: record.password.clone(),
                scope: Some(record.scope.clone()),
            },
            tenant: record
                .tenant_id
                .as_deref()
                .filter(|tenant| !tenant.is_empty())
                .unwrap_or("organizations")
                .to_owned(),
            authority_host: source
                .authority_host
                .clone()
                .unwrap_or_else(|| DEFAULT_AUTHORITY_HOST.to_owned()),
            transport: source.password_transport,
            clock: Arc::clone(&source.clock),
            state: Mutex::new(State::default()),
        }
    }

    fn lock(&self) -> Result<MutexGuard<'_, State>, LinkError> {
        self.state.lock().map_err(|_error| {
            LinkError::Authentication("the password token cache is poisoned".to_owned())
        })
    }

    // MSAL builds its application, authority included, once per credential
    fn authority(&self) -> Result<Authority, LinkError> {
        {
            let state = self.lock()?;
            if let Some(authority) = &state.authority {
                return Ok(authority.clone());
            }
        }
        let bridged = Bridged {
            transport: self.transport,
        };
        let authority = discover_authority(&bridged, &self.authority_host, &self.tenant, true)
            .map_err(|error| LinkError::Authentication(error.to_string()))?;
        self.lock()?.authority = Some(authority.clone());
        Ok(authority)
    }
}

impl Grant for EnvironmentPasswordGrant<'_> {
    fn token(&self) -> Result<ChainToken, LinkError> {
        let now = self.clock.now_unix_seconds();
        {
            let state = self.lock()?;
            if let Some(token) = &state.token
                && token.expires_on.is_some_and(|expires_on| {
                    expires_on.saturating_sub(now) >= REUSE_WINDOW_SECONDS
                })
            {
                return Ok(token.clone());
            }
        }
        let authority = self.authority()?;
        let response = token::request_token(self.transport, &authority, &self.input)
            .map_err(|error| LinkError::Authentication(error.to_string()))?;
        let token = password_token(&response, now)?;
        self.lock()?.token = Some(token.clone());
        Ok(token)
    }
}

fn password_token(response: &PasswordToken, now: i64) -> Result<ChainToken, LinkError> {
    let overflow =
        || LinkError::Authentication("the token lifetime overflows the clock".to_owned());
    let expires_on = now.checked_add(response.expires_in).ok_or_else(overflow)?;
    let refresh_on = match response.refresh_in {
        Some(refresh_in) => Some(now.checked_add(refresh_in).ok_or_else(overflow)?),
        None => response.refresh_on,
    };
    Ok(ChainToken {
        token: response.access_token.clone(),
        expires_on: Some(expires_on),
        token_type: Some(response.token_type.clone()),
        refresh_on,
    })
}
