use std::sync::Arc;

use betterlite_auth::trusted_network::request::{NetworkRequest, resolve_client_ip};
use betterlite_auth::trusted_network::settings::TrustedProxyConfig;
use betterlite_config::sections::login_throttle::Settings;
use betterlite_context::local_ttl_cache::CacheValue;
use tracing::warn;

use super::attempt::{Attempt, LoginAttempt, LoginRefused};
use super::error::LoginThrottleError;
use super::keys::{LoginKeys, UNKNOWN_SOURCE};
use super::local::{Clock, LocalStore, ThrottleStores};
use super::redis::{BlockTtls, FailureLimits, LoginRedis, ThrottleRedisError};
use super::source::{EXEMPT, declared_proxy_ranges, source_limit};

const REDIS_FALLBACK_WARNING: &str = "Redis failed while counting Admin UI sign-in attempts; using this worker's own counters until it recovers";

// the scope a block was found under, source first when that scope is on
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Scope {
    User,
    Source,
}

impl Scope {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Source => "source",
        }
    }
}

// the per-worker state every request in one process shares
#[derive(Clone)]
pub struct ThrottleRuntime {
    pub stores: ThrottleStores,
    pub clock: Clock,
    pub redis: Option<Arc<dyn LoginRedis>>,
}

// Failed-login limits for one request's source address, from LiteLLM's LoginThrottle record; source_limit is None when trusted_proxy_ranges is unset, since the peer may then be a shared ingress.
pub struct LoginThrottle {
    pub client_ip: String,
    pub source_limit: Option<i64>,
    pub user_limit: i64,
    pub window_seconds: i64,
    pub block_seconds: i64,
    pub enabled: bool,
    pub counters: Arc<dyn LocalStore>,
    pub blocks: Arc<dyn LocalStore>,
    pub clock: Clock,
    pub redis: Option<Arc<dyn LoginRedis>>,
}

impl LoginThrottle {
    #[must_use]
    pub fn from_request(
        request: &NetworkRequest<'_>,
        settings: &Settings,
        runtime: &ThrottleRuntime,
    ) -> Self {
        let proxies = declared_proxy_ranges(&settings.proxy_topology);
        let config = TrustedProxyConfig {
            use_forwarded_for: matches!(proxies.as_deref(), Some([_, ..])),
            trusted_proxy_cidrs: proxies.clone().unwrap_or_default(),
        };
        let (resolved, _via_trusted_proxy) = resolve_client_ip(request, &config);
        let known_source = resolved.is_some();
        let client_ip = resolved.unwrap_or_else(|| UNKNOWN_SOURCE.to_owned());
        let limit = source_limit(settings, &client_ip);
        let exempt = limit == EXEMPT;
        Self {
            source_limit: (proxies.is_some() && known_source && !exempt).then_some(limit),
            // user_limit_for: half the address allowance, rounded down, at least one
            user_limit: limit.saturating_div(2).max(1),
            window_seconds: settings.window_seconds,
            block_seconds: settings.block_seconds,
            enabled: !exempt && !settings.disabled,
            counters: Arc::clone(&runtime.stores.counters),
            blocks: Arc::clone(&runtime.stores.blocks),
            clock: Arc::clone(&runtime.clock),
            redis: runtime.redis.as_ref().map(Arc::clone),
            client_ip,
        }
    }

    // refuses a blocked key before any credential is looked at; otherwise hands back the attempt to settle
    pub async fn attempt(&self, username: &str) -> Result<Attempt<'_>, LoginThrottleError> {
        let attempt = LoginAttempt {
            throttle: self,
            username: username.to_owned(),
        };
        if !self.enabled {
            return Ok(Attempt::Allowed(attempt));
        }
        let keys = LoginKeys::of(&self.client_ip, username);
        let Some((scope, retry_after)) = self.active_block(&keys).await? else {
            return Ok(Attempt::Allowed(attempt));
        };
        let scope_name = scope.as_str();
        let client_ip = &self.client_ip;
        warn!(
            "Admin UI sign-in refused: the {scope_name} is blocked for {retry_after} more seconds; username=\"{username}\" source={client_ip}"
        );
        Ok(Attempt::Refused(LoginRefused { scope, retry_after }))
    }

    // _active_block: the per-scope maximum of the local and shared TTLs, source scope first when it is on
    async fn active_block(
        &self,
        keys: &LoginKeys,
    ) -> Result<Option<(Scope, i64)>, LoginThrottleError> {
        let local_user = self.local_block_ttl(&keys.pair_block)?;
        let local_source = self.local_block_ttl(&keys.source_block)?;
        let shared = self.shared_block_ttls(keys).await?;
        let user_ttl = local_user.max(shared.user);
        let source_ttl = local_source.max(shared.source);
        if self.source_limit.is_some() && source_ttl > 0 {
            return Ok(Some((Scope::Source, source_ttl)));
        }
        if user_ttl > 0 {
            return Ok(Some((Scope::User, user_ttl)));
        }
        Ok(None)
    }

    // _shared_block_ttls: transport failures warn and count as unblocked; a misread reply propagates
    async fn shared_block_ttls(&self, keys: &LoginKeys) -> Result<BlockTtls, LoginThrottleError> {
        let Some(redis) = &self.redis else {
            return Ok(BlockTtls::default());
        };
        match redis.block_ttls(keys).await {
            Ok(ttls) => Ok(ttls),
            Err(ThrottleRedisError::Unavailable { source }) => {
                warn!("{REDIS_FALLBACK_WARNING}: {source}");
                Ok(BlockTtls::default())
            }
            Err(error @ ThrottleRedisError::MalformedBlockTtls) => {
                Err(LoginThrottleError::Shared(error))
            }
        }
    }

    // _local_block_ttl: the stored expiry minus now, rounded up and clamped at zero
    fn local_block_ttl(&self, key: &str) -> Result<i64, LoginThrottleError> {
        let Some(stored) = self.blocks.get(key)? else {
            return Ok(0);
        };
        let expires_at = match stored {
            CacheValue::Integer(seconds) => num_traits::cast::<i64, f64>(seconds),
            CacheValue::Float(value) => Some(value),
            CacheValue::Null
            | CacheValue::Boolean(_)
            | CacheValue::Text(_)
            | CacheValue::Array(_)
            | CacheValue::Object(_)
            | CacheValue::Bytes(_)
            | CacheValue::Credential(_) => None,
        }
        .ok_or_else(|| LoginThrottleError::InvalidBlockExpiry {
            key: key.to_owned(),
        })?;
        let remaining = (*self.clock)().mul_add(-1.0_f64, expires_at).ceil();
        num_traits::cast::<f64, i64>(remaining)
            .map(|value| value.max(0))
            .ok_or_else(|| LoginThrottleError::InvalidBlockExpiry {
                key: key.to_owned(),
            })
    }

    // record_failure is not the enable gate: the pair counts first, and source only when this failure did not block the pair
    pub async fn record_failure(&self, username: &str) -> Result<BlockTtls, LoginThrottleError> {
        let keys = LoginKeys::of(&self.client_ip, username);
        let source_limit = self.source_limit.unwrap_or_default();
        if let Some(redis) = &self.redis {
            let limits = FailureLimits {
                user_limit: self.user_limit,
                source_limit,
                window_seconds: self.window_seconds,
                block_seconds: self.block_seconds,
            };
            match redis.record_failure(&keys, limits).await {
                Ok(ttls) => return Ok(ttls),
                Err(ThrottleRedisError::Unavailable { source }) => {
                    warn!("{REDIS_FALLBACK_WARNING}: {source}");
                }
                Err(error @ ThrottleRedisError::MalformedBlockTtls) => {
                    return Err(LoginThrottleError::Shared(error));
                }
            }
        }
        let user_block = self.local_bump(&keys.pair_counter, &keys.pair_block, self.user_limit)?;
        if source_limit == 0 || user_block > 0 {
            return Ok(BlockTtls {
                user: user_block,
                source: 0,
            });
        }
        let source_block =
            self.local_bump(&keys.source_counter, &keys.source_block, source_limit)?;
        Ok(BlockTtls {
            user: user_block,
            source: source_block,
        })
    }

    // _local_bump: a live block short-circuits, else the count goes up and the first count over the limit starts a fixed block, with the count's TTL set once as the Lua script does
    fn local_bump(
        &self,
        count_key: &str,
        block_key: &str,
        limit: i64,
    ) -> Result<i64, LoginThrottleError> {
        let blocked = self.local_block_ttl(block_key)?;
        if blocked > 0 {
            return Ok(blocked);
        }
        let count =
            num_traits::cast::<f64, i64>(self.counters.increment(count_key, self.window_seconds)?)
                .ok_or_else(|| LoginThrottleError::InvalidCount {
                    key: count_key.to_owned(),
                })?;
        if count <= limit {
            return Ok(0);
        }
        let seconds = num_traits::cast::<i64, f64>(self.block_seconds).ok_or(
            LoginThrottleError::UnrepresentableTtl {
                seconds: self.block_seconds,
            },
        )?;
        let now = (*self.clock)();
        self.blocks.set(
            block_key,
            CacheValue::Float(now.mul_add(1.0_f64, seconds)),
            self.block_seconds,
        )?;
        Ok(self.block_seconds)
    }

    // clear_pair: the shared counter goes first and a failure there only warns; the local one always goes
    pub async fn clear_pair(&self, username: &str) -> Result<(), LoginThrottleError> {
        let keys = LoginKeys::of(&self.client_ip, username);
        if let Some(redis) = &self.redis
            && let Err(error) = redis.delete_counter(&keys.pair_counter).await
        {
            warn!("{REDIS_FALLBACK_WARNING}: {error}");
        }
        self.counters.delete(&keys.pair_counter)?;
        Ok(())
    }
}
