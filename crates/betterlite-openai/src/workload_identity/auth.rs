use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use secrecy::SecretString;
use tokio::sync::Mutex as FlightMutex;

use super::config::SdkWorkloadIdentity;
use super::error::WorkloadIdentityError;
use super::exchange::{TOKEN_EXCHANGE_URL, TokenExchange, exchange_request, token_from_response};
use super::token_file::read_subject_token;
use super::transport::WorkloadIdentityTransport;

pub type MonotonicClock = Arc<dyn Fn() -> Duration + Send + Sync>;

const REFRESH_BUFFER: Duration = Duration::from_mins(20);

struct CachedToken {
    access_token: SecretString,
    expires_at: Duration,
    refresh_at: Duration,
}

enum Snapshot {
    Fresh(SecretString),
    Stale(SecretString),
    Unusable,
}

pub struct WorkloadIdentityAuth<T> {
    workload_identity: SdkWorkloadIdentity,
    exchange_url: String,
    transport: Arc<T>,
    clock: MonotonicClock,
    cached: Mutex<Option<CachedToken>>,
    // held by the one caller that is exchanging, so a caller that cannot take it waits for that exchange
    flight: FlightMutex<()>,
}

impl<T: WorkloadIdentityTransport> WorkloadIdentityAuth<T> {
    pub fn new(
        workload_identity: SdkWorkloadIdentity,
        transport: Arc<T>,
        clock: MonotonicClock,
    ) -> Self {
        Self {
            workload_identity,
            exchange_url: TOKEN_EXCHANGE_URL.to_owned(),
            transport,
            clock,
            cached: Mutex::new(None),
            flight: FlightMutex::new(()),
        }
    }

    pub async fn get_token(&self) -> Result<SecretString, WorkloadIdentityError> {
        loop {
            let arrival = self.snapshot();
            if let Snapshot::Fresh(token) = arrival {
                return Ok(token);
            }
            if let Ok(_flight) = self.flight.try_lock() {
                return self.lead_refresh().await;
            }
            drop(self.flight.lock().await);
            if !matches!(arrival, Snapshot::Unusable) {
                return self.usable_token();
            }
        }
    }

    async fn lead_refresh(&self) -> Result<SecretString, WorkloadIdentityError> {
        if let Snapshot::Fresh(token) = self.snapshot() {
            return Ok(token);
        }
        self.refresh().await?;
        self.usable_token()
    }

    fn usable_token(&self) -> Result<SecretString, WorkloadIdentityError> {
        match self.snapshot() {
            Snapshot::Fresh(token) | Snapshot::Stale(token) => Ok(token),
            Snapshot::Unusable => Err(WorkloadIdentityError::TokenUnusable),
        }
    }

    fn snapshot(&self) -> Snapshot {
        let now = (self.clock)();
        let cached = self.cached.lock().unwrap_or_else(PoisonError::into_inner);
        match cached.as_ref() {
            Some(token) if now < token.refresh_at.min(token.expires_at) => {
                Snapshot::Fresh(token.access_token.clone())
            }
            Some(token) if now < token.expires_at => Snapshot::Stale(token.access_token.clone()),
            _ => Snapshot::Unusable,
        }
    }

    async fn refresh(&self) -> Result<(), WorkloadIdentityError> {
        let subject_token = read_subject_token(&self.workload_identity.token_file).await?;
        let request = exchange_request(
            &self.exchange_url,
            &subject_token,
            &self.workload_identity.identity_provider_id,
            &self.workload_identity.service_account_id,
        )?;
        let response = self
            .transport
            .send_buffered(request)
            .await
            .map_err(WorkloadIdentityError::Transport)?;
        let exchange = token_from_response(&response)?;
        let now = (self.clock)();
        *self.cached.lock().unwrap_or_else(PoisonError::into_inner) =
            Some(cached_token(exchange, now));
        Ok(())
    }
}

// refresh_at is now + lifetime - min(1200s, lifetime / 2); a lifetime of zero or less is already expired
fn cached_token(exchange: TokenExchange, now: Duration) -> CachedToken {
    let lifetime = if exchange.expires_in <= 0.0_f64 {
        Duration::ZERO
    } else {
        Duration::try_from_secs_f64(exchange.expires_in).unwrap_or(Duration::MAX)
    };
    let buffer = REFRESH_BUFFER.min(lifetime.checked_div(2).unwrap_or(Duration::ZERO));
    CachedToken {
        access_token: exchange.access_token,
        expires_at: now.saturating_add(lifetime),
        refresh_at: now.saturating_add(lifetime.saturating_sub(buffer)),
    }
}
