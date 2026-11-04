#![cfg(test)]

use super::execution::{Batch, BatchLimits, Item, Mode};
use betterlite_context::call_context::{CallContext, CallInputs};
use std::{
    sync::Arc,
    time::{Duration, SystemTime},
};
use tokio::{
    sync::{Notify, oneshot},
    task::JoinSet,
};

struct Finished(Option<oneshot::Sender<()>>);
impl Drop for Finished {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _sent = sender.send(());
        }
    }
}

fn batch(values: Vec<u8>, mode: Mode) -> Batch<u8, CallContext<'static>> {
    let items = values
        .into_iter()
        .map(|value| Item {
            key: value.to_string(),
            request: value,
            context: CallContext::start(CallInputs {
                now: || SystemTime::UNIX_EPOCH,
                id: uuid::Uuid::nil,
                deadline: None,
                privacy: false,
            }),
        })
        .collect();
    Batch {
        items,
        mode,
        limits: BatchLimits {
            max_items: 8,
            max_bytes: 1024,
            concurrency: 2,
            deadline: Duration::from_secs(60),
        },
    }
}

#[rstest::rstest]
#[case::messages_keep_order(Mode::Ordered, false, vec![Ok(Some(0)), Ok(Some(1))])]
#[case::errors_keep_order(Mode::Ordered, true, vec![Err("failed"), Ok(Some(1))])]
#[case::deployments_first_success(Mode::Race, false, vec![Ok(Some(1))])]
#[case::deployments_skip_errors(Mode::Race, true, vec![Ok(Some(1))])]
#[tokio::test]
async fn batch_calls_batch_call_tests_behavior(
    #[case] mode: Mode,
    #[case] fail: bool,
    #[case] expected: Vec<Result<Option<u8>, &'static str>>,
) {
    let gate = Arc::new(Notify::new());
    let ordered = mode == Mode::Ordered;
    let mut owners = JoinSet::new();
    let release = Arc::clone(&gate);
    let mut input = batch(vec![0, 1], mode);
    if fail && mode == Mode::Race {
        input.limits.concurrency = 1;
    }
    let receiver = input
        .start(
            move |value, context| {
                let wait = Arc::clone(&release);
                async move {
                    assert_eq!(context.view().state.billing_time, SystemTime::UNIX_EPOCH);
                    if value == 0 && !fail {
                        wait.notified().await;
                    }
                    if value != 0 && ordered {
                        wait.notify_one();
                    }
                    if fail && value == 0 {
                        Err("failed")
                    } else {
                        Ok(Some(value))
                    }
                }
            },
            &mut owners,
        )
        .unwrap();
    let report = receiver.await.unwrap().unwrap();
    assert_eq!(report.results, expected);
    assert!(report.duplicates.is_empty());
    while let Some(joined) = owners.join_next().await {
        joined.unwrap();
    }
}

mod lifetime;
mod limits;
mod selection;

mod concurrency;
mod race;

mod edges;

mod merging;
