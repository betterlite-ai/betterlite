#![cfg(test)]
use crate::batch::{
    execution::{Batch, BatchError, Item, Mode},
    tests::batch,
};
use std::{collections::BTreeMap, error::Error as _};
use tokio::task::JoinSet;

#[rstest::rstest]
#[case::serialization_error_retains_source()]
#[tokio::test]
async fn batch_calls_batch_call_tests_behavior() {
    let input = Batch {
        items: vec![Item {
            key: "model".to_owned(),
            request: BTreeMap::from([(vec![1_u8], 2_u8)]),
            context: (),
        }],
        limits: batch(vec![], Mode::Ordered).limits,
        mode: Mode::Ordered,
    };
    let mut owners = JoinSet::new();
    let failure = input
        .start(
            |_, ()| async { Ok::<_, &'static str>(Some(1_u8)) },
            &mut owners,
        )
        .unwrap_err();
    assert!(matches!(failure, BatchError::Serialize(_)));
    assert!(failure.source().is_some());
    assert!(owners.is_empty());
}
