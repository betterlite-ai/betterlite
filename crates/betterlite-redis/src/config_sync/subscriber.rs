use std::fmt::Display;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use futures_util::future::{Either, select};
use tokio::sync::watch;

use super::clock::ConfigSyncClock;
use super::endpoint::{ConfigSyncMessage, ConfigSyncPubSubClient, ConfigSyncSubscription as _};
use super::error::ConfigSyncError;
use super::jitter::JitterSource;
use super::settings::ConfigSyncSettings;

pub type ResyncFuture<E> = Pin<Box<dyn Future<Output = Result<(), E>> + Send>>;
pub type ResyncCallback<E> = Arc<dyn Fn() -> ResyncFuture<E> + Send + Sync>;

pub struct ConfigSyncSubscriber<P, C, E> {
    pub client: P,
    pub clock: C,
    pub jitter: Arc<dyn JitterSource>,
    pub callbacks: Vec<ResyncCallback<E>>,
    pub settings: ConfigSyncSettings,
}

// LiteLLM's stop() cancels the task, so every wait ends as soon as the server signals shutdown
async fn wait_or_shutdown<W>(wait: W, shutdown: &mut watch::Receiver<bool>) -> bool
where
    W: Future<Output = ()> + Send,
{
    match select(Box::pin(wait), Box::pin(shutdown.changed())).await {
        Either::Left(((), _changed)) => false,
        Either::Right((_changed, _wait)) => true,
    }
}

impl<P, C, E> ConfigSyncSubscriber<P, C, E>
where
    P: ConfigSyncPubSubClient,
    C: ConfigSyncClock,
    E: Display + Send + 'static,
{
    pub async fn run(&self, mut shutdown: watch::Receiver<bool>) {
        let mut backoff = self.settings.backoff_initial;
        let mut last_resync_at = None;
        while !self
            .reconnect_once(&mut backoff, &mut last_resync_at, &mut shutdown)
            .await
        {
            // reconnect_once runs one session and returns true once shutdown is requested
        }
    }

    // returns true once the server has asked the subscriber to stop
    pub(super) async fn reconnect_once(
        &self,
        backoff: &mut Duration,
        last_resync_at: &mut Option<Duration>,
        shutdown: &mut watch::Receiver<bool>,
    ) -> bool {
        let subscribed = match select(
            Box::pin(self.client.subscribe()),
            Box::pin(shutdown.changed()),
        )
        .await
        {
            Either::Left((outcome, _changed)) => outcome,
            Either::Right((_changed, _subscribe)) => return true,
        };
        let outcome = match subscribed {
            Ok(mut subscription) => {
                *backoff = self.settings.backoff_initial;
                let outcome = self
                    .consume(&mut subscription, last_resync_at, shutdown)
                    .await;
                subscription.close().await;
                if *shutdown.borrow_and_update() {
                    return true;
                }
                outcome
            }
            Err(error) => Err(error),
        };
        let Some(error) = outcome.err() else {
            return false;
        };
        tracing::warn!(
            "config sync subscriber redis error: {error}; reconnecting in {:.0}s",
            backoff.as_secs_f64()
        );
        if wait_or_shutdown(self.clock.pause(*backoff), shutdown).await {
            return true;
        }
        *backoff = backoff.saturating_mul(2).min(self.settings.backoff_max);
        false
    }

    pub(super) async fn consume(
        &self,
        subscription: &mut P::Subscription,
        last_resync_at: &mut Option<Duration>,
        shutdown: &mut watch::Receiver<bool>,
    ) -> Result<(), ConfigSyncError> {
        loop {
            if *shutdown.borrow_and_update() {
                return Ok(());
            }
            let Some(outcome) = self
                .next_or_shutdown(subscription, self.settings.poll_timeout, shutdown)
                .await
            else {
                return Ok(());
            };
            let message = outcome?;
            if message.is_none() {
                continue;
            }
            let jitter =
                Duration::from_nanos(self.jitter.uniform_nanos(self.settings.jitter_max_nanos));
            let debounce = self.settings.debounce.saturating_add(jitter);
            if wait_or_shutdown(self.clock.pause(debounce), shutdown).await {
                return Ok(());
            }
            if self
                .wait_for_min_resync_interval(*last_resync_at, shutdown)
                .await
            {
                return Ok(());
            }
            self.drain_pending(subscription).await?;
            if *shutdown.borrow_and_update() {
                return Ok(());
            }
            self.run_resync_callbacks(shutdown).await;
            *last_resync_at = Some(self.clock.now());
        }
    }

    async fn next_or_shutdown(
        &self,
        subscription: &mut P::Subscription,
        timeout: Duration,
        shutdown: &mut watch::Receiver<bool>,
    ) -> Option<Result<Option<ConfigSyncMessage>, ConfigSyncError>> {
        match select(
            Box::pin(subscription.next_message(timeout)),
            Box::pin(shutdown.changed()),
        )
        .await
        {
            Either::Left((outcome, _changed)) => Some(outcome),
            Either::Right((_changed, _message)) => None,
        }
    }

    pub(super) async fn wait_for_min_resync_interval(
        &self,
        last_resync_at: Option<Duration>,
        shutdown: &mut watch::Receiver<bool>,
    ) -> bool {
        let Some(previous) = last_resync_at else {
            return false;
        };
        let elapsed = self.clock.now().saturating_sub(previous);
        let remaining = self.settings.min_resync_interval.saturating_sub(elapsed);
        if remaining.is_zero() {
            return false;
        }
        tracing::debug!(
            "config sync resync throttled for {:.1}s to cap fleet-wide reload rate",
            remaining.as_secs_f64()
        );
        wait_or_shutdown(self.clock.pause(remaining), shutdown).await
    }

    async fn drain_pending(
        &self,
        subscription: &mut P::Subscription,
    ) -> Result<(), ConfigSyncError> {
        while subscription.next_message(Duration::ZERO).await?.is_some() {}
        Ok(())
    }

    pub(super) async fn run_resync_callbacks(&self, shutdown: &mut watch::Receiver<bool>) {
        for callback in &self.callbacks {
            if !run_callback(callback, shutdown).await {
                return;
            }
        }
    }
}

async fn run_callback<E>(callback: &ResyncCallback<E>, shutdown: &mut watch::Receiver<bool>) -> bool
where
    E: Display,
{
    match select(callback(), Box::pin(shutdown.changed())).await {
        Either::Left((outcome, _changed)) => {
            if let Err(error) = outcome {
                tracing::warn!("config sync resync callback failed: {error}");
            }
            true
        }
        Either::Right((_changed, _callback)) => false,
    }
}
