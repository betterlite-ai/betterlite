use std::fmt::Display;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use super::clock::ConfigSyncClock;
use super::endpoint::{ConfigSyncPubSubClient, ConfigSyncSubscription as _};
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

impl<P, C, E> ConfigSyncSubscriber<P, C, E>
where
    P: ConfigSyncPubSubClient,
    C: ConfigSyncClock,
    E: Display + Send + 'static,
{
    pub async fn run(&self, shutting_down: &AtomicBool) {
        let mut backoff = self.settings.backoff_initial;
        let mut last_resync_at = None;
        while !shutting_down.load(Ordering::Acquire) {
            self.reconnect_once(&mut backoff, &mut last_resync_at, shutting_down)
                .await;
        }
    }

    pub(super) async fn reconnect_once(
        &self,
        backoff: &mut Duration,
        last_resync_at: &mut Option<Duration>,
        shutting_down: &AtomicBool,
    ) {
        let outcome = match self.client.subscribe().await {
            Ok(mut subscription) => {
                *backoff = self.settings.backoff_initial;
                let outcome = self
                    .consume(&mut subscription, last_resync_at, shutting_down)
                    .await;
                subscription.close().await;
                outcome
            }
            Err(error) => Err(error),
        };
        if let Err(error) = outcome {
            tracing::warn!(
                "config sync subscriber redis error: {error}; reconnecting in {:.0}s",
                backoff.as_secs_f64()
            );
            self.clock.pause(*backoff).await;
            *backoff = backoff.saturating_mul(2).min(self.settings.backoff_max);
        }
    }

    pub(super) async fn consume(
        &self,
        subscription: &mut P::Subscription,
        last_resync_at: &mut Option<Duration>,
        shutting_down: &AtomicBool,
    ) -> Result<(), ConfigSyncError> {
        loop {
            if shutting_down.load(Ordering::Acquire) {
                return Ok(());
            }
            let message = subscription
                .next_message(self.settings.poll_timeout)
                .await?;
            if message.is_none() {
                continue;
            }
            let jitter =
                Duration::from_nanos(self.jitter.uniform_nanos(self.settings.jitter_max_nanos));
            self.clock
                .pause(self.settings.debounce.saturating_add(jitter))
                .await;
            self.wait_for_min_resync_interval(*last_resync_at).await;
            self.drain_pending(subscription).await?;
            self.run_resync_callbacks().await;
            *last_resync_at = Some(self.clock.now());
        }
    }

    pub(super) async fn wait_for_min_resync_interval(&self, last_resync_at: Option<Duration>) {
        let Some(previous) = last_resync_at else {
            return;
        };
        let elapsed = self.clock.now().saturating_sub(previous);
        let remaining = self.settings.min_resync_interval.saturating_sub(elapsed);
        if remaining.is_zero() {
            return;
        }
        tracing::debug!(
            "config sync resync throttled for {:.1}s to cap fleet-wide reload rate",
            remaining.as_secs_f64()
        );
        self.clock.pause(remaining).await;
    }

    async fn drain_pending(
        &self,
        subscription: &mut P::Subscription,
    ) -> Result<(), ConfigSyncError> {
        while subscription.next_message(Duration::ZERO).await?.is_some() {}
        Ok(())
    }

    pub(super) async fn run_resync_callbacks(&self) {
        for callback in &self.callbacks {
            if let Err(error) = callback().await {
                tracing::warn!("config sync resync callback failed: {error}");
            }
        }
    }
}
