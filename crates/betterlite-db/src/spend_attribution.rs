pub mod access_groups;
pub mod enqueue;
pub mod tags;

#[path = "spend_attribution/support.rs"]
mod support;

use crate::spend_queues::SpendQueueError;

// A failed enqueue call, named by the entity it was charging; the writer logs it and, except for team members and access groups, passes it up.
#[derive(Debug, thiserror::Error)]
pub enum SpendAttributionError {
    #[error("the {entity} spend update could not be queued")]
    Enqueue {
        entity: &'static str,
        #[source]
        source: SpendQueueError,
    },
    #[error("the request tags could not be read")]
    Tags {
        #[from]
        source: tags::EnqueueTagsError,
    },
}
