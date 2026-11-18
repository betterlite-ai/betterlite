#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Mutex;

use betterlite_models::model_registration::JsonObject;

use super::access_groups::ServedDeployments;
use super::enqueue::{EnqueueFuture, EntitySpendQueue};
use crate::spend_queues::SpendQueueError;
use crate::spend_queues::entity::SpendUpdateQueueItem;

// An owned bounded sink: it records what the writer queued and fails the enqueue at the chosen position, like a closed or full queue.
pub(super) struct RecordingQueue {
    items: Mutex<Vec<SpendUpdateQueueItem>>,
    fail_at: Option<usize>,
}

impl RecordingQueue {
    pub(super) fn new(fail_at: Option<usize>) -> Self {
        Self {
            items: Mutex::new(Vec::new()),
            fail_at,
        }
    }

    pub(super) fn items(&self) -> Vec<SpendUpdateQueueItem> {
        self.items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl EntitySpendQueue for RecordingQueue {
    fn enqueue(&self, update: SpendUpdateQueueItem) -> EnqueueFuture<'_> {
        Box::pin(async move {
            let mut items = self
                .items
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if self.fail_at == Some(items.len()) {
                return Err(SpendQueueError::ZeroCapacity);
            }
            items.push(update);
            Ok(())
        })
    }
}

// An immutable deployment fixture: a deployment id maps to the model_info mapping the router would return for it.
pub(super) struct FixtureDeployments {
    deployments: BTreeMap<String, JsonObject>,
}

impl FixtureDeployments {
    pub(super) fn new(entries: &[(&str, &str)]) -> Self {
        let deployments = entries
            .iter()
            .map(|(id, model_info)| {
                let parsed = serde_json::from_str::<JsonObject>(model_info)
                    .expect("the fixture model_info is a JSON object");
                ((*id).to_owned(), parsed)
            })
            .collect();
        Self { deployments }
    }
}

impl ServedDeployments for FixtureDeployments {
    fn model_info(&self, deployment_id: &str) -> Option<&JsonObject> {
        self.deployments.get(deployment_id)
    }
}
