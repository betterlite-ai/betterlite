pub mod deployment;
pub mod error;
pub mod keys;
mod pre_call;
pub mod success;
mod tests;

use std::slice;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::router_observations::counter::{CounterSink as _, Increment};
use crate::routing_state::counter::RouterCounters;
use crate::routing_state::delta::DeltaStore;

use self::deployment::DeploymentFacts;
use self::error::RpmLimitExceeded;
use self::pre_call::{Rejection, past_limit, reserve};
use self::success::{SuccessEvent, success_increment};

// BaseRoutingStrategy.__init__(should_batch_redis_writes=True, default_sync_interval=0.1)
pub const SYNC_INTERVAL: Duration = Duration::from_millis(100);
pub const SHOULD_BATCH_WRITES: bool = true;

// lowest_tpm_rpm_v2's rpm reservation and its success callback over one counters owner
pub struct UsageV2Accounting<S: DeltaStore + 'static> {
    pub counters: Arc<RouterCounters<S>>,
    pub ttl: Duration,
}

impl<S: DeltaStore + 'static> UsageV2Accounting<S> {
    // async_pre_call_check: the local count decides the boundary, then the batched increment carries the request
    pub async fn pre_call(
        &self,
        deployment: &DeploymentFacts<'_>,
        now: DateTime<Utc>,
    ) -> Result<(), RpmLimitExceeded> {
        let rejection = Rejection::async_for(deployment);
        let reserved = reserve(&self.counters, deployment, now, &rejection)?;
        let counter = self
            .counters
            .increment_window_value(&reserved.key, Decimal::ONE, self.ttl)
            .await;
        match counter {
            Ok(updated) => past_limit(deployment, reserved.limit, updated, &rejection),
            Err(error) => {
                tracing::warn!(%error, "the rpm window increment failed; the request continues");
                Ok(())
            }
        }
    }

    // pre_call_check: the sync facade stages the same reservation through the sink, then reads the count
    // back; its rejection carries neither a retry header nor num_retries
    pub fn pre_call_now(
        &self,
        deployment: &DeploymentFacts<'_>,
        now: DateTime<Utc>,
    ) -> Result<(), RpmLimitExceeded> {
        let rejection = Rejection::default();
        let reserved = reserve(&self.counters, deployment, now, &rejection)?;
        let increment = Increment {
            key: reserved.key.clone(),
            value: 1,
            ttl: self.ttl,
        };
        if let Err(error) = self
            .counters
            .increment_pipeline(slice::from_ref(&increment))
        {
            tracing::warn!(%error, "the rpm counter increment failed; the request continues");
            return Ok(());
        }
        let Some(updated) = self.counters.read(&reserved.key) else {
            return Ok(());
        };
        past_limit(
            deployment,
            reserved.limit,
            Decimal::from(updated),
            &rejection,
        )
    }

    // async_log_success_event: the tokens ride the batched queue and reach the store on a sync
    pub async fn log_success(
        &self,
        event: &SuccessEvent<'_>,
        now: DateTime<Utc>,
    ) -> Option<String> {
        let increment = success_increment(event, now, self.ttl)?;
        let value = Decimal::from(increment.value);
        let incremented = self
            .counters
            .increment_window_value(&increment.key, value, increment.ttl)
            .await;
        let Err(error) = incremented else {
            return Some(increment.key);
        };
        tracing::warn!(%error, "the success token increment failed; the callback continues");
        None
    }

    // log_success_event: the synchronous callback stages the same increment through the sync sink
    pub fn log_success_now(&self, event: &SuccessEvent<'_>, now: DateTime<Utc>) -> Option<String> {
        let increment = success_increment(event, now, self.ttl)?;
        match self
            .counters
            .increment_pipeline(slice::from_ref(&increment))
        {
            Ok(()) => Some(increment.key),
            Err(error) => {
                tracing::warn!(%error, "the success token increment failed; the callback continues");
                None
            }
        }
    }
}
