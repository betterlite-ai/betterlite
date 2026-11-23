#![cfg(test)]

use std::error::Error as _;
use std::io;
use std::sync::Mutex;

use async_trait::async_trait;
use betterlite_anthropic::native_compaction::protocol::{CompactionProtocol, Mapping};
use betterlite_models::decisions::Json;
use rstest::rstest;
use tokio::task::JoinSet;

use super::{
    CompactionExecutor, CompactionExecutorError, is_native_compaction_call, native_compaction_call,
    native_compaction_parent,
};

#[derive(Debug, PartialEq, Eq)]
struct Seen {
    protocol: CompactionProtocol,
    payload: Mapping,
    parent_model: Option<String>,
    logging_disabled: bool,
}

struct RecordingExecutor {
    seen: Mutex<Option<Seen>>,
}

#[async_trait]
impl CompactionExecutor for RecordingExecutor {
    async fn execute(
        &self,
        protocol: CompactionProtocol,
        payload: &Mapping,
        parent_model: Option<&str>,
        logging_disabled: bool,
    ) -> Result<Mapping, CompactionExecutorError> {
        *self.seen.lock().unwrap() = Some(Seen {
            protocol,
            payload: payload.clone(),
            parent_model: parent_model.map(str::to_owned),
            logging_disabled,
        });
        Ok(payload.clone())
    }
}

#[test]
fn outside_a_scope_is_not_a_native_call() {
    assert!(!is_native_compaction_call());
    assert_eq!(native_compaction_parent("claude-compactor"), None);
}

#[tokio::test]
async fn a_scope_without_a_pair_sets_only_the_child_flag() {
    native_compaction_call(None, None, async {
        assert!(is_native_compaction_call());
        assert_eq!(native_compaction_parent("claude-compactor"), None);
    })
    .await;
}

#[tokio::test]
async fn a_pair_answers_its_own_compactor_with_the_parent() {
    native_compaction_call(Some("gpt-parent"), Some("claude-compactor"), async {
        assert!(is_native_compaction_call());
        assert_eq!(
            native_compaction_parent("claude-compactor"),
            Some("gpt-parent".to_owned())
        );
        assert_eq!(native_compaction_parent("other"), None);
    })
    .await;
}

#[rstest]
#[case::empty_parent(Some(""), Some("claude-compactor"))]
#[case::empty_compactor(Some("gpt-parent"), Some(""))]
#[case::both_empty(Some(""), Some(""))]
#[case::missing_parent(None, Some("claude-compactor"))]
#[case::missing_compactor(Some("gpt-parent"), None)]
#[tokio::test]
async fn a_missing_or_empty_name_leaves_the_pair_empty(
    #[case] parent_model: Option<&str>,
    #[case] compactor: Option<&str>,
) {
    native_compaction_call(parent_model, compactor, async {
        assert!(is_native_compaction_call());
        assert_eq!(native_compaction_parent("claude-compactor"), None);
        assert_eq!(native_compaction_parent(""), None);
    })
    .await;
}

#[tokio::test]
async fn an_inner_scope_without_a_pair_hides_the_outer_pair() {
    native_compaction_call(Some("gpt-parent"), Some("claude-compactor"), async {
        assert_eq!(
            native_compaction_parent("claude-compactor"),
            Some("gpt-parent".to_owned())
        );
        native_compaction_call(None, None, async {
            assert!(is_native_compaction_call());
            assert_eq!(native_compaction_parent("claude-compactor"), None);
        })
        .await;
        assert_eq!(
            native_compaction_parent("claude-compactor"),
            Some("gpt-parent".to_owned())
        );
    })
    .await;
    assert!(!is_native_compaction_call());
    assert_eq!(native_compaction_parent("claude-compactor"), None);
}

#[tokio::test]
async fn a_spawned_task_does_not_inherit_the_scope() {
    native_compaction_call(Some("gpt-parent"), Some("claude-compactor"), async {
        let mut tasks = JoinSet::new();
        let _task = tasks.spawn(async {
            let outside = (
                is_native_compaction_call(),
                native_compaction_parent("claude-compactor"),
            );
            let inside =
                native_compaction_call(Some("gpt-parent"), Some("claude-compactor"), async {
                    (
                        is_native_compaction_call(),
                        native_compaction_parent("claude-compactor"),
                    )
                })
                .await;
            (outside, inside)
        });
        let (outside, inside) = tasks.join_next().await.unwrap().unwrap();
        assert_eq!(outside, (false, None));
        assert_eq!(inside, (true, Some("gpt-parent".to_owned())));
    })
    .await;
}

#[rstest]
#[case::chat(CompactionProtocol::Chat, None, false)]
#[case::messages(CompactionProtocol::Messages, Some("gpt-parent"), true)]
#[tokio::test]
async fn an_executor_receives_the_protocol_payload_parent_and_flag(
    #[case] protocol: CompactionProtocol,
    #[case] parent_model: Option<&str>,
    #[case] logging_disabled: bool,
) {
    let recording = RecordingExecutor {
        seen: Mutex::new(None),
    };
    let executor: &dyn CompactionExecutor = &recording;
    let payload = Mapping::from_iter([(
        "model".to_owned(),
        Json::Text("claude-compactor".to_owned()),
    )]);
    let returned = executor
        .execute(protocol, &payload, parent_model, logging_disabled)
        .await
        .unwrap();
    assert_eq!(returned, payload);
    let seen = recording.seen.lock().unwrap().take().unwrap();
    assert_eq!(seen.protocol, protocol);
    assert_eq!(seen.payload, payload);
    assert_eq!(seen.parent_model, parent_model.map(str::to_owned));
    assert_eq!(seen.logging_disabled, logging_disabled);
}

#[test]
fn an_executor_error_shows_and_returns_its_source() {
    let error = CompactionExecutorError(Box::new(io::Error::other("the child request failed")));
    assert_eq!(error.to_string(), "the child request failed");
    assert_eq!(
        error.source().unwrap().to_string(),
        "the child request failed"
    );
}
