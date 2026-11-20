#![cfg(test)]

use std::sync::Arc;
use std::sync::Mutex;

use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_redis::config_sync::publish::PublishChange;

use super::AGENTS_OBJECT_TYPE;
use super::ConfigSyncAgentPublisher;
use crate::agent_persistence::AgentChangePublisher;

fn recording_publisher(journal: Arc<Mutex<Vec<String>>>) -> PublishChange<TableError> {
    Arc::new(move |object_type| {
        let journal = Arc::clone(&journal);
        let name = object_type.to_owned();
        Box::pin(async move {
            journal.lock().expect("the journal lock").push(name);
            Ok(())
        })
    })
}

#[tokio::test]
async fn change_publisher_sends_one_litellm_agentstable_message() {
    let journal = Arc::new(Mutex::new(Vec::new()));
    let publisher = ConfigSyncAgentPublisher {
        publish: recording_publisher(Arc::clone(&journal)),
    };
    publisher.agents_changed().await;
    assert_eq!(
        *journal.lock().expect("the journal lock"),
        vec![String::from(AGENTS_OBJECT_TYPE)]
    );
    assert_eq!(AGENTS_OBJECT_TYPE, "litellm_agentstable");
    publisher.agents_changed().await;
    assert_eq!(journal.lock().expect("the journal lock").len(), 2);
}

#[tokio::test]
async fn change_publisher_keeps_running_after_a_failed_publish() {
    let publish: PublishChange<TableError> = Arc::new(|_object_type| {
        Box::pin(async {
            Err(TableError::Publish {
                message: String::from("down"),
            })
        })
    });
    let publisher = ConfigSyncAgentPublisher { publish };
    publisher.agents_changed().await;
}
