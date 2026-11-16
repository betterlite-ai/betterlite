mod tests;

use std::marker::PhantomData;
use std::time::Duration;

use async_trait::async_trait;
use tokio::time::{Instant, sleep, timeout};

use super::distributed_lock::{DistributedLock, LockAcquisition};
use super::{RefreshCoordinator, RefreshStep, TokenKey};

pub const DEFAULT_KEY_PREFIX: &str = "mcp:refresh_lock:";
pub const DEFAULT_LOCK_TTL: Duration = Duration::from_secs(10);
pub const DEFAULT_REFRESH_BUDGET: Duration = Duration::from_secs(20);
pub const DEFAULT_WAIT_TIMEOUT: Duration = Duration::from_secs(35);
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Clone, Debug)]
pub struct RedisTunables {
    pub key_prefix: String,
    pub lock_ttl: Duration,
    pub refresh_budget: Duration,
    pub wait_timeout: Duration,
    pub poll_interval: Duration,
}

impl Default for RedisTunables {
    fn default() -> Self {
        Self {
            key_prefix: DEFAULT_KEY_PREFIX.to_owned(),
            lock_ttl: DEFAULT_LOCK_TTL,
            refresh_budget: DEFAULT_REFRESH_BUDGET,
            wait_timeout: DEFAULT_WAIT_TIMEOUT,
            poll_interval: DEFAULT_POLL_INTERVAL,
        }
    }
}

pub struct RedisRefreshCoordinator<Lock, Output, Error> {
    pub lock: Lock,
    pub tunables: RedisTunables,
    pub new_token: fn() -> String,
    pub marker: PhantomData<fn() -> (Output, Error)>,
}

impl<Lock, Output, Error> RedisRefreshCoordinator<Lock, Output, Error> {
    #[must_use]
    pub fn new(lock: Lock) -> Self {
        Self {
            lock,
            tunables: RedisTunables::default(),
            new_token: || uuid::Uuid::new_v4().simple().to_string(),
            marker: PhantomData,
        }
    }
}

#[async_trait]
impl<Lock, Output, Error> RefreshCoordinator for RedisRefreshCoordinator<Lock, Output, Error>
where
    Lock: DistributedLock,
    Output: Clone + Send + 'static,
    Error: Clone + Send + 'static,
{
    type Output = Output;
    type Error = Error;

    async fn run(
        &self,
        key: TokenKey,
        refresh: RefreshStep<'_, Output, Error>,
        reread: RefreshStep<'_, Output, Error>,
    ) -> Result<Output, Error> {
        let lock_key = format!(
            "{}{}:{}",
            self.tunables.key_prefix, key.user_id, key.server_id
        );
        let token = (self.new_token)();
        match self
            .lock
            .acquire(&lock_key, &token, self.tunables.lock_ttl)
            .await
        {
            LockAcquisition::Acquired => self.refresh_holding(&lock_key, &token, refresh).await,
            LockAcquisition::Error => refresh().await,
            LockAcquisition::Held => {
                self.wait_for_holder(&lock_key).await;
                reread().await
            }
        }
    }
}

impl<Lock: DistributedLock, Output, Error> RedisRefreshCoordinator<Lock, Output, Error> {
    async fn refresh_holding(
        &self,
        lock_key: &str,
        token: &str,
        refresh: RefreshStep<'_, Output, Error>,
    ) -> Result<Output, Error> {
        let started = Instant::now();
        let half_ttl = self.tunables.lock_ttl.checked_div(2).unwrap_or_default();
        let mut flight = refresh();
        let outcome = loop {
            if started.elapsed() >= self.tunables.refresh_budget {
                break flight.await;
            }
            if let Ok(result) = timeout(half_ttl, &mut flight).await {
                break result;
            }
            if !self
                .lock
                .extend(lock_key, token, self.tunables.lock_ttl)
                .await
            {
                break flight.await;
            }
        };
        self.lock.release(lock_key, token).await;
        outcome
    }

    // A loser waits out the holder's bounded lease, then reads the token the holder persisted.
    async fn wait_for_holder(&self, lock_key: &str) {
        let started = Instant::now();
        while started.elapsed() < self.tunables.wait_timeout {
            if !self.lock.is_held(lock_key).await {
                break;
            }
            sleep(self.tunables.poll_interval).await;
        }
    }
}
