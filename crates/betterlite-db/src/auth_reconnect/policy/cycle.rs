use std::sync::atomic::Ordering;

use secrecy::{ExposeSecret as _, SecretString};
use tokio::time::timeout;
use tracing::{debug, error, info, warn};

use super::AuthReconnect;
use crate::auth_reconnect::clock::ReconnectClock;
use crate::auth_reconnect::engine::{EngineState, ReconnectEngine};
use crate::auth_reconnect::lock::ReconnectLock;
use crate::auth_reconnect::request::{ReconnectError, ReconnectRequest};
use crate::database_refresh::handle::PoolFuture;

pub(super) async fn run_cycle<Engine, Clock, Gate>(
    policy: &AuthReconnect<Engine, Clock, Gate>,
    request: &ReconnectRequest<'_>,
) -> Result<(), ReconnectError<Engine::Error>>
where
    Engine: ReconnectEngine,
    Clock: ReconnectClock,
    Gate: ReconnectLock,
{
    let limit = request.timeout.unwrap_or(policy.settings.watchdog_timeout);
    let expected_generation = policy.engine.writer().generation;
    let dead =
        policy.dead.load(Ordering::SeqCst) || policy.engine.engine_state() == EngineState::Dead;
    let cycle: PoolFuture<'_, (), ReconnectError<Engine::Error>> = if dead {
        warn!("A database engine was confirmed dead; reconnecting.");
        policy.engine.reap();
        policy.engine.cleanup_watcher();
        Box::pin(reconnect(policy, request, expected_generation, true))
    } else {
        debug!("Performing Prisma DB reconnect (engine alive or unknown).");
        Box::pin(reconnect(policy, request, expected_generation, false))
    };
    let outcome = match timeout(limit, cycle).await {
        Ok(outcome) => outcome,
        Err(_elapsed) => Err(ReconnectError::Timeout),
    };
    if dead && outcome.is_ok() {
        policy.dead.store(false, Ordering::SeqCst);
    }
    outcome
}

fn database_url<'request, Engine>(
    request: &'request ReconnectRequest<'_>,
    missing: &str,
) -> Result<&'request SecretString, ReconnectError<Engine::Error>> {
    request
        .database_url
        .filter(|url| !url.expose_secret().is_empty())
        .ok_or_else(|| {
            error!("{}", missing);
            ReconnectError::MissingDatabaseUrl
        })
}

async fn reconnect<Engine, Clock, Gate>(
    policy: &AuthReconnect<Engine, Clock, Gate>,
    request: &ReconnectRequest<'_>,
    expected_generation: u64,
    dead: bool,
) -> Result<(), ReconnectError<Engine::Error>>
where
    Engine: ReconnectEngine,
    Clock: ReconnectClock,
    Gate: ReconnectLock,
{
    let missing = if dead {
        "DATABASE_URL not set; cannot recreate the database pool."
    } else {
        "DATABASE_URL not set; cannot reconnect the database pool."
    };
    let url = database_url(request, missing)?;
    if !dead && !request.force_recreate && !writer_needs_recreate(policy).await? {
        return Ok(());
    }
    if !dead {
        policy.engine.cleanup_watcher();
    }
    let recreated = policy
        .engine
        .recreate(url, Some(expected_generation))
        .await
        .map_err(ReconnectError::Engine)?;
    policy
        .engine
        .rearm_watcher()
        .await
        .map_err(ReconnectError::Engine)?;
    if !dead {
        policy
            .engine
            .writer_select_one()
            .await
            .map_err(ReconnectError::Engine)?;
    }
    if request.force_recreate && !recreated {
        if dead {
            policy.dead.store(false, Ordering::SeqCst);
        }
        return Err(ReconnectError::ForcedDecline);
    }
    Ok(())
}

async fn writer_needs_recreate<Engine, Clock, Gate>(
    policy: &AuthReconnect<Engine, Clock, Gate>,
) -> Result<bool, ReconnectError<Engine::Error>>
where
    Engine: ReconnectEngine,
    Clock: ReconnectClock,
    Gate: ReconnectLock,
{
    match policy.engine.writer_read_only().await {
        Ok(true) => {
            warn!(
                "The writer answers the probe with a read-only session; recreating the database pool."
            );
            Ok(true)
        }
        Ok(false) => {
            info!("The writer is healthy on the probe; skipping the recreate.");
            policy.engine.mark_writer_recovered();
            policy
                .engine
                .rearm_watcher()
                .await
                .map_err(ReconnectError::Engine)?;
            Ok(false)
        }
        Err(error) => {
            warn!(error = %error, "The writer probe failed; recreating the database pool.");
            Ok(true)
        }
    }
}
