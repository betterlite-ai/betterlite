#![cfg(test)]
use crate::batch::{
    execution::{Batch, Item, Mode},
    tests::batch,
};
use tokio::task::JoinSet;

#[rstest::rstest]
#[case::vllm_uses_per_item_executor("vllm/test")]
#[tokio::test]
async fn batch_calls_batch_call_tests_behavior(#[case] model: &'static str) {
    let items = ["first", "second"]
        .into_iter()
        .map(|message| Item {
            key: message.to_owned(),
            request: (model, message),
            context: (),
        })
        .collect();
    let input = Batch {
        items,
        limits: batch(vec![], Mode::Ordered).limits,
        mode: Mode::Ordered,
    };
    let mut owners = JoinSet::new();
    let receiver = input
        .start(
            |request, ()| async move { Ok::<_, &'static str>(Some(request)) },
            &mut owners,
        )
        .unwrap();
    assert_eq!(
        receiver.await.unwrap().unwrap().results,
        vec![Ok(Some((model, "first"))), Ok(Some((model, "second"))),]
    );
    while let Some(joined) = owners.join_next().await {
        joined.unwrap();
    }
}
