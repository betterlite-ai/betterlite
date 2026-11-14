use std::sync::atomic::Ordering;

use secrecy::ExposeSecret as _;
use tracing::{debug, error, info, warn};

use super::AuthReconnect;
use crate::auth_reconnect::clock::ReconnectClock;
use crate::auth_reconnect::engine::{EngineState, ReconnectEngine};
use crate::auth_reconnect::lock::ReconnectLock;
use crate::auth_reconnect::request::{ReconnectError, ReconnectRequest};
use crate::database_refresh::handle::PoolFuture;

impl<Engine, Clock, Gate> AuthReconnect<Engine, Clock, Gate>
where
    Engine: ReconnectEngine,
    Clock: ReconnectClock,
    Gate: ReconnectLock,
{
    pub(super) async fn run_cycle(
        &self,
        request: &ReconnectRequest<'_>,
    ) -> Result<(), ReconnectError<Engine::Error>> {
        let limit = request.timeout.unwrap_or(self.settings.watchdog_timeout);
        let expected_generation = self.engine.writer().generation;
        let dead =
            self.dead.load(Ordering::SeqCst) || self.engine.engine_state() == EngineState::Dead;
        let cycle: PoolFuture<'_, (), ReconnectError<Engine::Error>> = if dead {
            warn!("A database engine was confirmed dead; reconnecting.");
            self.engine.reap();
            self.engine.cleanup_watcher();
            Box::pin(self.heavy(request, expected_generation))
        } else {
            debug!("Performing Prisma DB reconnect (engine alive or unknown).");
            Box::pin(self.direct(request, expected_generation))
        };
        let outcome = match tokio::time::timeout(limit, cycle).await {
            Ok(outcome) => outcome,
            Err(_elapsed) => Err(ReconnectError::Timeout),
        };
        if dead && outcome.is_ok() {
            self.dead.store(false, Ordering::SeqCst);
        }
        outcome
    }

    async fn heavy(
        &self,
        request: &ReconnectRequest<'_>,
        expected_generation: u64,
    ) -> Result<(), ReconnectError<Engine::Error>> {
        let Some(url) = request
            .database_url
            .filter(|url| !url.expose_secret().is_empty())
        else {
            error!("DATABASE_URL not set; cannot recreate the database pool.");
            return Err(ReconnectError::MissingDatabaseUrl);
        };
        let recreated = self
            .engine
            .recreate(url, Some(expected_generation))
            .await
            .map_err(ReconnectError::Engine)?;
        self.engine
            .rearm_watcher()
            .await
            .map_err(ReconnectError::Engine)?;
        if request.force_recreate && !recreated {
            self.dead.store(false, Ordering::SeqCst);
            return Err(ReconnectError::ForcedDecline);
        }
        Ok(())
    }

    async fn direct(
        &self,
        request: &ReconnectRequest<'_>,
        expected_generation: u64,
    ) -> Result<(), ReconnectError<Engine::Error>> {
        let Some(url) = request
            .database_url
            .filter(|url| !url.expose_secret().is_empty())
        else {
            error!("DATABASE_URL not set; cannot reconnect the database pool.");
            return Err(ReconnectError::MissingDatabaseUrl);
        };
        if !request.force_recreate && !self.writer_needs_recreate().await? {
            return Ok(());
        }
        self.engine.cleanup_watcher();
        let recreated = self
            .engine
            .recreate(url, Some(expected_generation))
            .await
            .map_err(ReconnectError::Engine)?;
        self.engine
            .rearm_watcher()
            .await
            .map_err(ReconnectError::Engine)?;
        self.engine
            .writer_select_one()
            .await
            .map_err(ReconnectError::Engine)?;
        if request.force_recreate && !recreated {
            return Err(ReconnectError::ForcedDecline);
        }
        Ok(())
    }

    async fn writer_needs_recreate(&self) -> Result<bool, ReconnectError<Engine::Error>> {
        match self.engine.writer_read_only().await {
            Ok(true) => {
                warn!(
                    "The writer answers the probe with a read-only session; recreating the database pool."
                );
                Ok(true)
            }
            Ok(false) => {
                info!("The writer is healthy on the probe; skipping the recreate.");
                self.engine.mark_writer_recovered();
                self.engine
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
}
