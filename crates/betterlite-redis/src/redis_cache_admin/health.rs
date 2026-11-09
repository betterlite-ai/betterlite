use std::time::Instant;

use super::RedisAdminError;
use crate::cache_primitives::protocol::AsyncRedisCommands;
use crate::redis_client_reuse::service_event::ServiceEvent;
use crate::redis_client_reuse::service_hooks::ServiceHooks;

pub(crate) const ASYNC_PING: &str = "async_ping";

pub async fn ping_commands<C>(
    commands: &C,
    hooks: &dyn ServiceHooks,
    caller: Option<&str>,
) -> Result<bool, RedisAdminError>
where
    C: AsyncRedisCommands + Sync,
    RedisAdminError: From<C::Error>,
{
    let started = Instant::now();
    let reply = commands.ping().await;
    let event = ServiceEvent {
        call_type: ASYNC_PING,
        duration: started.elapsed(),
        key: caller.map(str::to_owned),
    };
    match reply {
        Ok(answer) => {
            hooks.success(&event);
            Ok(answer)
        }
        Err(source) => {
            let error = RedisAdminError::from(source);
            hooks.failure(&event, &error);
            tracing::error!("LiteLLM Redis Cache PING: - Got exception from REDIS : {error}");
            Err(error)
        }
    }
}
