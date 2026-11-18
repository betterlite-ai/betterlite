use chrono::{DateTime, Utc};
use rand::Rng;

use crate::deployment_identity::spec::DeploymentSpec;
use crate::routing_read_batch::local::LocalTier;
use crate::routing_read_batch::store::BatchReadStore;
use crate::routing_read_batch::{
    ReadTiers, RequestReads, RoutingReadBatch, RoutingReadSession, UsageReadKeys,
};
use crate::routing_value_store::RouteValue;

use super::error::SelectionError;
use super::selection::{CounterKeys, Selection};

// async_get_available_deployments and get_available_deployments: one pool, the counters read for it and the
// request's token estimate.
pub struct UsageRoutingV2<'request> {
    pub group: &'request str,
    pub deployments: &'request [DeploymentSpec],
    pub input_tokens: i64,
    pub now: DateTime<Utc>,
}

impl UsageRoutingV2<'_> {
    // async_get_available_deployments: the tpm and the rpm windows of the whole pool in one batch read, taken
    // from the request's prefetched usage when its healthy-deployment read already fetched them.
    pub async fn select_async<S, R>(
        &self,
        store: &S,
        tiers: &ReadTiers<RouteValue>,
        request: &mut RequestReads<RouteValue>,
        random: &mut R,
    ) -> Result<usize, SelectionError>
    where
        S: BatchReadStore<Value = RouteValue> + Sync,
        R: Rng,
    {
        let keys = CounterKeys::for_deployments(self.deployments, self.now)?;
        let tpm_len = keys.tpm.len();
        // RoutingReadBatch::for_strategy answers this batch for every usage read it is given
        let mut batch: RoutingReadBatch<RouteValue> = RoutingReadBatch {
            usage: Some(UsageReadKeys {
                tpm: keys.tpm,
                rpm: keys.rpm,
            }),
            prefetched_usage: None,
        };
        let results = {
            let mut session = RoutingReadSession {
                store,
                tiers,
                request,
            };
            batch.read_routing_state(&mut session, &[]).await
        };
        // combined() lays the tpm windows first, and a failed whole read leaves both sides unknown
        let (tpm, rpm) = match results.usage.as_deref() {
            Some(values) => (values.get(..tpm_len), values.get(tpm_len..)),
            None => (None, None),
        };
        let selection = Selection {
            group: self.group,
            deployments: self.deployments,
            input_tokens: self.input_tokens,
            tpm,
            rpm,
        };
        match selection.choose(random)? {
            Some(index) => Ok(index),
            None => Err(selection.no_deployment()?.into()),
        }
    }

    // get_available_deployments: the sync facade reads the tpm windows and then the rpm windows separately, which
    // here is the two reads of the local tier this process holds, the only cache a sync caller can reach.
    pub fn select_now<R>(
        &self,
        tier: Option<&LocalTier<RouteValue>>,
        random: &mut R,
    ) -> Result<usize, SelectionError>
    where
        R: Rng,
    {
        let keys = CounterKeys::for_deployments(self.deployments, self.now)?;
        let tpm = tier.map(|tier| tier.batch_get(&keys.tpm));
        let rpm = tier.map(|tier| tier.batch_get(&keys.rpm));
        let selection = Selection {
            group: self.group,
            deployments: self.deployments,
            input_tokens: self.input_tokens,
            tpm: tpm.as_deref(),
            rpm: rpm.as_deref(),
        };
        match selection.choose(random)? {
            Some(index) => Ok(index),
            None => Err(selection.no_deployment()?.into()),
        }
    }
}
