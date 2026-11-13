#![cfg(test)]
use crate::batch::{execution::Mode, tests::batch};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{task::JoinSet, time::sleep};

#[rstest::rstest]
#[case::concurrency_one(1)]
#[case::concurrency_two(2)]
#[case::duplicate_entries_are_distinct(3)]
#[tokio::test(start_paused = true)]
async fn batch_calls_batch_call_tests_behavior(#[case] concurrency: usize) {
    let mut input = batch(vec![1, 1, 1, 1], Mode::Ordered);
    input.limits.concurrency = concurrency;
    let active = Arc::new(AtomicUsize::new(0));
    let maximum = Arc::new(AtomicUsize::new(0));
    let (running, peak) = (Arc::clone(&active), Arc::clone(&maximum));
    let mut owners = JoinSet::new();
    let receiver = input
        .start(
            move |value, _| {
                let (count, highest) = (Arc::clone(&running), Arc::clone(&peak));
                async move {
                    let previous = count.fetch_add(1, Ordering::SeqCst);
                    highest.fetch_max(previous, Ordering::SeqCst);
                    sleep(Duration::from_secs(1)).await;
                    count.fetch_sub(1, Ordering::SeqCst);
                    Ok::<_, &'static str>(Some(value))
                }
            },
            &mut owners,
        )
        .unwrap();
    let report = receiver.await.unwrap().unwrap();
    assert_eq!(report.results, vec![Ok(Some(1)); 4]);
    assert_eq!(
        maximum.load(Ordering::SeqCst),
        concurrency.checked_sub(1).unwrap()
    );
    assert_eq!(active.load(Ordering::SeqCst), 0);
    let positions: Vec<_> = report
        .duplicates
        .iter()
        .map(|entry| (entry.key.as_str(), entry.first, entry.second))
        .collect();
    assert_eq!(positions, vec![("1", 0, 1), ("1", 0, 2), ("1", 0, 3)]);
    while let Some(joined) = owners.join_next().await {
        joined.unwrap();
    }
}
