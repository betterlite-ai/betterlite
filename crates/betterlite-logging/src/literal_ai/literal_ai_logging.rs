pub mod graphql;
pub mod ids;
pub mod payload;
pub mod prepare;
pub mod send;
pub mod settings;
pub mod step;

mod tests;

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};

use self::ids::IdSource;
use self::payload::CallbackKwargs;
use self::prepare::{StepError, prepare_log_data};
use self::send::{BatchSender, SendError, post_steps};
use self::settings::{Overrides, Settings, SettingsError};
use self::step::Step;
use crate::batch::logger::BatchQueue;
use crate::http_analytics_loggers::delivery::Delivery;

// The litellm.integrations.literal_ai.LiteralAILogger callback surface, without the response object it never reads.
#[derive(Clone, Copy, Debug)]
pub struct Call<'event> {
    pub kwargs: &'event CallbackKwargs,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
}

pub struct LiteralAILogger<D, I> {
    pub settings: Settings,
    pub queue: BatchQueue<Step>,
    pub delivery: D,
    pub ids: I,
}

#[derive(Clone, Copy)]
enum Mode {
    Sync,
    Async,
}

impl<D, I> LiteralAILogger<D, I>
where
    D: Delivery,
    I: IdSource,
{
    pub fn from_environment(
        environment: &BTreeMap<String, String>,
        overrides: Overrides,
        delivery: D,
        ids: I,
    ) -> Result<Self, SettingsError> {
        let settings = Settings::from_environment(environment, overrides)?;
        let queue = BatchQueue::new(settings.batch);
        Ok(Self {
            settings,
            queue,
            delivery,
            ids,
        })
    }

    pub async fn log_success_event(&self, call: Call<'_>) {
        if let Err(error) = self.record_and_send(call, Mode::Sync).await {
            tracing::error!(%error, "Literal AI Layer Error - error logging success event.");
        }
    }

    pub async fn log_failure_event(&self, call: Call<'_>) {
        tracing::info!("Literal AI Failure Event Logging!");
        if let Err(error) = self.record_and_send(call, Mode::Sync).await {
            tracing::error!(%error, "Literal AI Layer Error - error logging failure event.");
        }
    }

    pub async fn async_log_success_event(&self, call: Call<'_>) {
        if let Err(error) = self.record_and_send(call, Mode::Async).await {
            tracing::error!(%error, "Literal AI Layer Error - error logging async success event.");
        }
    }

    pub async fn async_log_failure_event(&self, call: Call<'_>) {
        tracing::info!("Literal AI Failure Event Logging!");
        if let Err(error) = self.record_and_send(call, Mode::Async).await {
            tracing::error!(%error, "Literal AI Layer Error - error logging async failure event.");
        }
    }

    // flush_queue clears the whole queue whatever the send answered, matching the base class's list.clear().
    pub async fn flush_queue(&self) {
        let sender = BatchSender {
            settings: &self.settings,
            delivery: &self.delivery,
        };
        let report = self.queue.flush(&sender).await;
        tracing::debug!(batch = report.batch, "Literal AI flushed a batch of steps");
    }

    // _send_batch reads the live queue and never clears it, so the next threshold resends what it already sent.
    pub async fn send_batch(&self) -> Result<(), SendError<D::Error>> {
        let steps = self.queue.pending();
        if steps.is_empty() {
            return Ok(());
        }
        post_steps(&self.delivery, &self.settings, &steps).await
    }

    async fn record_and_send(&self, call: Call<'_>, mode: Mode) -> Result<(), StepError> {
        let step = prepare_log_data(call.kwargs, call.start_time, call.end_time, &self.ids)?;
        let queued = self.queue.enqueue(step);
        if queued.depth < self.settings.batch.batch_size {
            return Ok(());
        }
        match mode {
            Mode::Sync => self.send_sync_batch().await,
            Mode::Async => self.flush_queue().await,
        }
        Ok(())
    }

    async fn send_sync_batch(&self) {
        let Err(error) = self.send_batch().await else {
            return;
        };
        if let SendError::Status { status, body } = &error {
            tracing::error!("Literal AI Error: {status} - {body}");
        } else {
            tracing::error!("Literal AI Layer Error: {error}");
        }
    }
}
