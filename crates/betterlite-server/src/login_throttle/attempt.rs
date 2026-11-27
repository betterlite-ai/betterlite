use std::collections::BTreeMap;

use betterlite_error::proxy_error_types::ProxyErrorTypes;
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use tracing::warn;

use super::error::LoginThrottleError;
use super::throttle::{LoginThrottle, Scope};

const TOO_MANY_ATTEMPTS_MESSAGE: &str = "Too many failed sign-in attempts. Try again later.";

// the outcome of attempt: a refusal carries the block that stopped it, ready as LiteLLM's 429
pub enum Attempt<'throttle> {
    Allowed(LoginAttempt<'throttle>),
    Refused(LoginRefused),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LoginRefused {
    pub scope: Scope,
    pub retry_after: i64,
}

impl LoginRefused {
    #[must_use]
    pub fn exception(&self) -> ProxyException {
        let mut headers = BTreeMap::new();
        headers.insert("Retry-After".to_owned(), self.retry_after.to_string());
        ProxyException::new(ProxyExceptionInit {
            message: TOO_MANY_ATTEMPTS_MESSAGE.to_owned(),
            error_type: ProxyErrorTypes::AuthError.to_string(),
            param: Some("username".to_owned()),
            code: ExceptionCode::Status(429),
            headers,
            openai_code: None,
            provider_specific_fields: BTreeMap::new(),
        })
    }
}

// LiteLLM's LoginAttempt: the credential check's two endings, both no-ops for a disabled throttle
pub struct LoginAttempt<'throttle> {
    throttle: &'throttle LoginThrottle,
    username: String,
}

impl LoginAttempt<'_> {
    pub async fn succeeded(self) -> Result<(), LoginThrottleError> {
        if !self.throttle.enabled {
            return Ok(());
        }
        self.throttle.clear_pair(&self.username).await
    }

    pub async fn failed(self) -> Result<(), LoginThrottleError> {
        if !self.throttle.enabled {
            return Ok(());
        }
        let ttls = self.throttle.record_failure(&self.username).await?;
        if ttls.user == 0 && ttls.source == 0 {
            return Ok(());
        }
        let scope = if ttls.user != 0 {
            Scope::User
        } else {
            Scope::Source
        };
        let seconds = if ttls.user != 0 {
            ttls.user
        } else {
            ttls.source
        };
        let scope_name = scope.as_str();
        let username = &self.username;
        let client_ip = &self.throttle.client_ip;
        warn!(
            "Admin UI sign-in blocked for {seconds} seconds after too many failures; scope={scope_name} username=\"{username}\" source={client_ip}"
        );
        Ok(())
    }
}
